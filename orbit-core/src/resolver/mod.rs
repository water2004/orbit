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

use crate::lockfile::{OrbitLockfile, PackageEntry};
use crate::manifest::OrbitManifest;
use crate::metadata::Environment;
use crate::resolver::graph::{build_solver_graph, build_solver_graph_for_target};
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
    check_lockfile_graph_with_loader(manifest, lockfile, None)
}

pub(crate) fn check_lockfile_graph_with_loader(
    manifest: &OrbitManifest,
    lockfile: &OrbitLockfile,
    loader_package: Option<&types::PlatformCandidate>,
) -> Result<(), String> {
    let graph = build_solver_graph(manifest, lockfile, &HashMap::new(), loader_package);
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
        build_solver_graph_for_target(manifest, lockfile, &HashMap::new(), loader_package, target);
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
    catalog: &CandidateCatalog,
) -> Result<ResolutionPortfolio, String> {
    let graph = build_solver_graph(
        manifest,
        lockfile,
        &catalog.candidates,
        catalog.loader_package.as_ref(),
    );
    let mut maximized_mods: Vec<_> = catalog
        .candidates
        .keys()
        .chain(lockfile.packages.iter().map(|entry| &entry.mod_id))
        .cloned()
        .collect();
    maximized_mods.sort();
    maximized_mods.dedup();
    let maximized_packages = maximized_mods.into_iter().map(SolverPackage::logical);
    let watched_candidates = highest_candidates(&catalog.candidates, &manifest.project.modloader);
    let mut trace = diagnostics::ResolutionTrace::new(watched_candidates);
    let solutions = match pubgrub::resolve_maximal_solutions_with_observer(
        &graph.provider,
        graph.root_package.clone(),
        graph.root_version.clone(),
        maximized_packages,
        |version| {
            version.domain().map_or_else(
                || Ranges::strictly_higher_than(version.clone()),
                |semantic| solver_range(Ranges::strictly_higher_than(semantic.clone())),
            )
        },
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
            .max_by_key(|(_, entry)| Version::parse(&entry.version, context.loader))
            .expect("installed is not empty");
        let current = Version::parse(&active.version, context.loader);
        let selected = Version::parse(selected_version, context.loader);
        let kind = match selected.cmp(&current) {
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
            Version::parse(&candidate.jar_version, context.loader) > *selected
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

fn highest_candidates(
    candidates: &HashMap<String, Vec<CandidateVersion>>,
    loader: &str,
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
[platform]
minecraft_jar = { path = "minecraft.jar", sha256 = "test" }
loader_jar = { path = "loader.jar", sha256 = "test" }
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
            id: format!("candidate-{version}"),
            filename: format!("candidate-{version}.jar"),
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
[dependencies]
reeses-sodium-options = "*"
voxy = "*"
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
    async fn a_different_candidate_at_the_same_declared_version_is_not_an_upgrade() {
        let mut current = lockfile();
        current.packages.retain(|entry| entry.mod_id == "a");
        let mut only_a = manifest();
        only_a.dependencies.shift_remove("b");
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
        only_a.dependencies.shift_remove("b");

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
        manifest.dependencies.shift_remove("b");
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

[dependencies]
iris = "*"
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
                1
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

        let error = select_resolution(portfolio, Some(Box::new(|_| 2))).unwrap_err();

        assert_eq!(
            error,
            "dependency solution selector returned invalid choice 3 for 2 alternatives"
        );
    }
}
