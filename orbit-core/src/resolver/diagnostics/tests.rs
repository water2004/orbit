use pubgrub::{Ranges, resolve_with_observer};

use super::*;
use crate::resolver::provider::OrbitDependencyProvider;
use crate::resolver::types::{CandidateDiagnosticKind, SolverPackage, SolverVersion};
use crate::versions::Version;

fn domain_version(value: &str) -> Version {
    Version::Generic(value.to_string())
}

fn version(value: &str) -> SolverVersion {
    domain_version(value).into()
}

fn package(value: &str) -> SolverPackage {
    SolverPackage::logical(value)
}

#[test]
fn explains_a_candidate_excluded_before_selection() {
    let mut provider = OrbitDependencyProvider::new();
    provider.add_package_versions(package("root"), vec![version("1")]);
    provider.add_package_versions(package("a"), vec![version("2"), version("1")]);
    provider.add_package_versions(package("b"), vec![version("1")]);
    provider.add_package_deps(
        package("root"),
        version("1"),
        vec![
            (package("a"), Ranges::full()),
            (package("b"), Ranges::singleton(version("1"))),
        ],
    );
    provider.add_package_deps(package("a"), version("2"), vec![]);
    provider.add_package_deps(package("a"), version("1"), vec![]);
    provider.add_package_deps(
        package("b"),
        version("1"),
        vec![(package("a"), Ranges::singleton(version("1")))],
    );

    let mut trace = ResolutionTrace::with_progress([("a".to_string(), version("2"))], None);
    let solution =
        resolve_with_observer(&provider, package("root"), version("1"), &mut trace).unwrap();

    assert_eq!(solution.get(&package("a")), Some(&version("1")));
    let diagnostic = trace.into_solutions()[0].diagnose_skipped("a", &version("1"));
    assert_eq!(
        diagnostic.kind,
        CandidateDiagnosticKind::ExcludedByPropagation
    );
    assert!(
        diagnostic
            .facts
            .iter()
            .any(|fact| fact == "b 1 requires a 1")
    );
}

#[test]
fn explains_a_candidate_rejected_after_conflicting_choices() {
    let mut provider = OrbitDependencyProvider::new();
    provider.add_package_versions(package("root"), vec![version("1")]);
    provider.add_package_versions(package("a"), vec![version("2"), version("1")]);
    provider.add_package_versions(package("b"), vec![version("2"), version("1")]);
    provider.add_package_deps(
        package("root"),
        version("1"),
        vec![
            (package("a"), Ranges::full()),
            (package("b"), Ranges::singleton(version("1"))),
        ],
    );
    provider.add_package_deps(
        package("a"),
        version("2"),
        vec![(package("b"), Ranges::singleton(version("2")))],
    );
    provider.add_package_deps(
        package("a"),
        version("1"),
        vec![(package("b"), Ranges::singleton(version("1")))],
    );
    provider.add_package_deps(package("b"), version("2"), vec![]);
    provider.add_package_deps(package("b"), version("1"), vec![]);

    let mut trace = ResolutionTrace::with_progress([("a".to_string(), version("2"))], None);
    let solution =
        resolve_with_observer(&provider, package("root"), version("1"), &mut trace).unwrap();

    assert_eq!(solution.get(&package("a")), Some(&version("1")));
    let diagnostic = trace.into_solutions()[0].diagnose_skipped("a", &version("1"));
    assert!(
        matches!(
            diagnostic.kind,
            CandidateDiagnosticKind::ExcludedByPropagation | CandidateDiagnosticKind::Backtracked
        ),
        "{diagnostic:?}"
    );
    assert!(diagnostic.facts.iter().any(|fact| fact.contains('a')));
    assert!(diagnostic.facts.iter().any(|fact| fact.contains('b')));
}
