use pubgrub::{Ranges, resolve_with_observer};

use super::*;
use crate::resolver::provider::OrbitDependencyProvider;

fn version(value: &str) -> Version {
    Version::Generic(value.to_string())
}

#[test]
fn explains_a_candidate_excluded_before_selection() {
    let mut provider = OrbitDependencyProvider::new();
    provider.add_package_versions("root".to_string(), vec![version("1")]);
    provider.add_package_versions("a".to_string(), vec![version("2"), version("1")]);
    provider.add_package_versions("b".to_string(), vec![version("1")]);
    provider.add_package_deps(
        "root".to_string(),
        version("1"),
        vec![
            ("a".to_string(), Ranges::full()),
            ("b".to_string(), Ranges::singleton(version("1"))),
        ],
    );
    provider.add_package_deps("a".to_string(), version("2"), vec![]);
    provider.add_package_deps("a".to_string(), version("1"), vec![]);
    provider.add_package_deps(
        "b".to_string(),
        version("1"),
        vec![("a".to_string(), Ranges::singleton(version("1")))],
    );

    let mut trace = ResolutionTrace::new([("a".to_string(), version("2"))]);
    let solution =
        resolve_with_observer(&provider, "root".to_string(), version("1"), &mut trace).unwrap();

    assert_eq!(solution.get(&"a".to_string()), Some(&version("1")));
    let message = trace.describe_skipped("a", &version("1"));
    assert!(message.contains("excluded by dependency propagation"));
    assert!(message.contains("b 1 requires a 1"));
}

#[test]
fn explains_a_candidate_discarded_by_backtracking() {
    let mut provider = OrbitDependencyProvider::new();
    provider.add_package_versions("root".to_string(), vec![version("1")]);
    provider.add_package_versions("a".to_string(), vec![version("2"), version("1")]);
    provider.add_package_versions("b".to_string(), vec![version("2"), version("1")]);
    provider.add_package_deps(
        "root".to_string(),
        version("1"),
        vec![
            ("a".to_string(), Ranges::full()),
            ("b".to_string(), Ranges::singleton(version("1"))),
        ],
    );
    provider.add_package_deps(
        "a".to_string(),
        version("2"),
        vec![("b".to_string(), Ranges::singleton(version("2")))],
    );
    provider.add_package_deps(
        "a".to_string(),
        version("1"),
        vec![("b".to_string(), Ranges::singleton(version("1")))],
    );
    provider.add_package_deps("b".to_string(), version("2"), vec![]);
    provider.add_package_deps("b".to_string(), version("1"), vec![]);

    let mut trace = ResolutionTrace::new([("a".to_string(), version("2"))]);
    let solution =
        resolve_with_observer(&provider, "root".to_string(), version("1"), &mut trace).unwrap();

    assert_eq!(solution.get(&"a".to_string()), Some(&version("1")));
    let message = trace.describe_skipped("a", &version("1"));
    assert!(message.contains("tried, then backtracked"));
    assert!(message.contains("a"));
    assert!(message.contains("b"));
}

#[test]
fn explains_when_provider_order_prefers_another_allowed_version() {
    let mut provider = OrbitDependencyProvider::new();
    provider.add_package_versions("root".to_string(), vec![version("1")]);
    provider.add_package_versions("a".to_string(), vec![version("1"), version("2")]);
    provider.add_package_deps(
        "root".to_string(),
        version("1"),
        vec![("a".to_string(), Ranges::full())],
    );
    provider.add_package_deps("a".to_string(), version("1"), vec![]);
    provider.add_package_deps("a".to_string(), version("2"), vec![]);

    let mut trace = ResolutionTrace::new([("a".to_string(), version("2"))]);
    let solution =
        resolve_with_observer(&provider, "root".to_string(), version("1"), &mut trace).unwrap();

    assert_eq!(solution.get(&"a".to_string()), Some(&version("1")));
    let message = trace.describe_skipped("a", &version("1"));
    assert!(message.contains("was allowed"));
    assert!(message.contains("version selection preferred 1"));
}
