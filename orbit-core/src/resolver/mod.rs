//! Dependency resolution orchestration and public resolver utilities.

mod catalog;
mod constraints;
mod diagnostics;
mod graph;
mod local;
mod ordering;
mod provider;
pub mod types;

use std::collections::{BTreeMap, HashMap};

use pubgrub::Ranges;

use crate::lockfile::{OrbitLockfile, PackageEntry};
use crate::manifest::OrbitManifest;
use crate::metadata::Environment;
use crate::providers::ModProvider;
use crate::resolver::catalog::{CatalogRequest, complete_candidate_catalog};
use crate::resolver::graph::{build_solver_graph, build_solver_graph_for_target};
use crate::resolver::ordering::resolution_warnings;
use crate::resolver::types::{
    CandidateCatalog, CandidateVersion, ResolutionPortfolio, ResolutionReport, ResolutionSelector,
    SolverPackage, SolverVersion,
};
use crate::versions::Version;

pub(crate) use graph::is_platform_package;

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
        selector
            .map(|select| select(&portfolio.alternatives))
            .unwrap_or(0)
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
) -> Result<pubgrub::SelectedDependencies<SolverPackage, SolverVersion>, String> {
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

/// Complete the candidate graph and enumerate every single-package-maximal solution.
pub async fn resolve_candidate_portfolio(
    manifest: &OrbitManifest,
    lockfile: &OrbitLockfile,
    catalog: &mut CandidateCatalog,
    providers: &[Box<dyn ModProvider>],
) -> Result<ResolutionPortfolio, String> {
    let initial_graph = build_solver_graph(manifest, lockfile, &catalog.candidates);
    complete_candidate_catalog(CatalogRequest {
        catalog,
        lockfile,
        providers,
        minecraft_version: &manifest.project.mc_version,
        loader: &manifest.project.modloader,
        exclusions: &initial_graph.exclusions,
        target: initial_graph.target,
    })
    .await?;

    let graph = build_solver_graph(manifest, lockfile, &catalog.candidates);
    let mut maximized_mods: Vec<_> = catalog
        .candidates
        .keys()
        .chain(lockfile.packages.iter().map(|entry| &entry.mod_id))
        .cloned()
        .collect();
    maximized_mods.sort();
    maximized_mods.dedup();
    let maximized_packages = maximized_mods.into_iter().map(SolverPackage::top_level);
    let watched_candidates = highest_candidates(&catalog.candidates, &manifest.project.modloader);
    let mut trace = diagnostics::ResolutionTrace::new(watched_candidates);
    let solutions = match pubgrub::resolve_maximal_solutions_with_observer(
        &graph.provider,
        graph.root_package.clone(),
        graph.root_version.clone(),
        maximized_packages,
        |version| Ranges::strictly_higher_than(version.clone()),
        &mut trace,
    ) {
        Ok(solutions) => solutions,
        Err(pubgrub::PubGrubError::NoSolution(derivation_tree)) => {
            return Err(diagnostics::describe_no_solution(&derivation_tree));
        }
        Err(pubgrub::PubGrubError::ErrorChoosingVersion { package, source: _ }) => {
            return Err(format!(
                "internal error: no version of '{package}' matches constraint"
            ));
        }
        Err(pubgrub::PubGrubError::ErrorRetrievingDependencies {
            package,
            version,
            source,
        }) => {
            return Err(format!(
                "internal error: deps of '{package}' v{version}: {source}"
            ));
        }
        Err(pubgrub::PubGrubError::ErrorInShouldCancel(error)) => {
            return Err(error.to_string());
        }
    };
    let snapshots = trace.into_solutions();
    if snapshots.len() != solutions.len() {
        return Err("internal error: solver trace count does not match solution count".to_string());
    }

    let report_context = ReportContext {
        lockfile,
        candidates: &catalog.candidates,
        loader: &manifest.project.modloader,
        exclusions: &graph.exclusions,
        overrides: &graph.overrides,
        target: graph.target,
    };
    let alternatives = solutions
        .into_iter()
        .zip(snapshots)
        .map(|(solution, snapshot)| collect_report(&report_context, &solution, &snapshot))
        .collect();
    Ok(ResolutionPortfolio { alternatives })
}

struct ReportContext<'a> {
    lockfile: &'a OrbitLockfile,
    candidates: &'a HashMap<String, Vec<CandidateVersion>>,
    loader: &'a str,
    exclusions: &'a graph::ExclusionMap,
    overrides: &'a graph::OverrideMap,
    target: Environment,
}

fn collect_report(
    context: &ReportContext<'_>,
    solution: &pubgrub::SelectedDependencies<SolverPackage, SolverVersion>,
    trace: &diagnostics::ResolutionSnapshot,
) -> ResolutionReport {
    let warnings = resolution_warnings(
        context.lockfile,
        context.candidates,
        solution,
        context.loader,
        context.exclusions,
        context.overrides,
        context.target,
    );

    let mut upgrades = BTreeMap::new();
    let mut diagnostics = Vec::new();
    let mut packages: Vec<_> = context.candidates.keys().collect();
    packages.sort();
    for package in packages {
        let Some(selected) = solution.get(&SolverPackage::top_level(package)) else {
            continue;
        };
        let current = context
            .lockfile
            .find(package)
            .map(|entry| entry.version.as_str())
            .unwrap_or("?");
        let selected_version = selected.to_string();
        if selected_version != current {
            upgrades.insert(package.clone(), selected_version);
            continue;
        }

        let Some(candidate) = highest_candidate(
            context.candidates.get(package).map(Vec::as_slice),
            context.loader,
        ) else {
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

fn highest_candidates(
    candidates: &HashMap<String, Vec<CandidateVersion>>,
    loader: &str,
) -> impl Iterator<Item = (String, Version)> {
    let mut watched: Vec<_> = candidates
        .iter()
        .filter_map(|(package, versions)| {
            highest_candidate(Some(versions.as_slice()), loader).map(|candidate| {
                (
                    package.clone(),
                    Version::parse(&candidate.jar_version, loader),
                )
            })
        })
        .collect();
    watched.sort_by(|(left, _), (right, _)| left.cmp(right));
    watched.into_iter()
}

fn highest_candidate<'a>(
    candidates: Option<&'a [CandidateVersion]>,
    loader: &str,
) -> Option<&'a CandidateVersion> {
    candidates?
        .iter()
        .max_by_key(|candidate| Version::parse(&candidate.jar_version, loader))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lockfile::{LockMeta, PackageEntry};
    use crate::metadata::{Environment, ModDependency};

    fn manifest() -> OrbitManifest {
        toml::from_str(
            r#"
[project]
name = "test"
mc_version = "1.20.1"
modloader = "forge"
modloader_version = "47.2.0"
[dependencies]
a = "*"
b = "*"
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
            provider: "file".to_string(),
            modrinth: None,
            curseforge: None,
            file: None,
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

        let portfolio = resolve_candidate_portfolio(&manifest(), &lockfile(), &mut catalog, &[])
            .await
            .unwrap();
        let upgrades: std::collections::BTreeSet<_> = portfolio
            .alternatives
            .iter()
            .map(|alternative| alternative.upgrades.clone())
            .collect();

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
    async fn independent_upgrades_have_one_solution_and_do_not_prompt() {
        let mut catalog = CandidateCatalog::default();
        for package in ["a", "b"] {
            catalog.candidates.insert(
                package.to_string(),
                vec![candidate("1", Vec::new()), candidate("2", Vec::new())],
            );
        }
        let portfolio = resolve_candidate_portfolio(&manifest(), &lockfile(), &mut catalog, &[])
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
            selected.upgrades,
            BTreeMap::from([
                ("a".to_string(), "2".to_string()),
                ("b".to_string(), "2".to_string()),
            ])
        );
    }

    #[test]
    fn multiple_solutions_use_the_selected_alternative() {
        let first = ResolutionReport {
            upgrades: BTreeMap::from([("a".to_string(), "2".to_string())]),
            ..ResolutionReport::default()
        };
        let second = ResolutionReport {
            upgrades: BTreeMap::from([("b".to_string(), "2".to_string())]),
            ..ResolutionReport::default()
        };
        let portfolio = ResolutionPortfolio {
            alternatives: vec![first, second.clone()],
        };

        let selected = select_resolution(
            portfolio,
            Some(Box::new(|alternatives| {
                assert_eq!(alternatives.len(), 2);
                1
            })),
        )
        .unwrap();

        assert_eq!(selected.upgrades, second.upgrades);
    }

    #[test]
    fn invalid_solution_selection_is_rejected() {
        let portfolio = ResolutionPortfolio {
            alternatives: vec![ResolutionReport::default(), ResolutionReport::default()],
        };

        let error = select_resolution(portfolio, Some(Box::new(|_| 2))).unwrap_err();

        assert_eq!(
            error,
            "dependency solution selector returned invalid choice 3 for 2 alternatives"
        );
    }
}
