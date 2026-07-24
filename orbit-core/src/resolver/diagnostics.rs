use std::collections::HashMap;

use pubgrub::{DerivationTree, External, Ranges, SolverEvent, SolverObserver};

use crate::versions::Version;

type Cause = DerivationTree<String, Ranges<Version>, String>;

#[derive(Debug, Clone)]
enum SkippedVersionReason {
    ExcludedByPropagation(Cause),
    Backtracked(Cause),
    ProviderPreferred(Version),
}

#[derive(Debug)]
struct WatchedVersion {
    version: Version,
    decision_level: Option<u32>,
    reason: Option<SkippedVersionReason>,
}

/// Records why candidate versions were skipped during the successful solver run.
pub(crate) struct ResolutionTrace {
    watched: HashMap<String, WatchedVersion>,
}

impl ResolutionTrace {
    pub(crate) fn new(candidates: impl IntoIterator<Item = (String, Version)>) -> Self {
        Self {
            watched: candidates
                .into_iter()
                .map(|(package, version)| {
                    (
                        package,
                        WatchedVersion {
                            version,
                            decision_level: None,
                            reason: None,
                        },
                    )
                })
                .collect(),
        }
    }

    pub(crate) fn describe_skipped(&self, package: &str, selected: &Version) -> String {
        let Some(watched) = self.watched.get(package) else {
            return format!("    {package} stayed at {selected}; no candidate trace was recorded");
        };

        let mut output = match &watched.reason {
            Some(SkippedVersionReason::ExcludedByPropagation(_)) => format!(
                "    {package} stayed at {selected}; candidate {} was excluded by dependency propagation:",
                watched.version
            ),
            Some(SkippedVersionReason::Backtracked(_)) => format!(
                "    {package} stayed at {selected}; candidate {} was tried, then backtracked after a conflict:",
                watched.version
            ),
            Some(SkippedVersionReason::ProviderPreferred(preferred)) => {
                return format!(
                    "    {package} stayed at {selected}; candidate {} was allowed, but version selection preferred {preferred}",
                    watched.version
                );
            }
            None => {
                return format!(
                    "    {package} stayed at {selected}; candidate {} was not selected, but no excluding derivation was recorded",
                    watched.version
                );
            }
        };

        let cause = match watched.reason.as_ref() {
            Some(SkippedVersionReason::ExcludedByPropagation(cause))
            | Some(SkippedVersionReason::Backtracked(cause)) => cause,
            _ => unreachable!(),
        };
        let facts = external_facts(cause);
        if facts.is_empty() {
            let mut packages: Vec<_> = cause.packages().into_iter().cloned().collect();
            packages.sort();
            output.push_str(&format!(
                "\n      - conflict involved {}",
                packages.join(", ")
            ));
            return output;
        }

        const MAX_FACTS: usize = 8;
        for fact in facts.iter().take(MAX_FACTS) {
            output.push_str("\n      - ");
            output.push_str(fact);
        }
        if facts.len() > MAX_FACTS {
            output.push_str(&format!(
                "\n      - and {} more dependency fact(s)",
                facts.len() - MAX_FACTS
            ));
        }
        output
    }
}

impl SolverObserver<String, Ranges<Version>, String> for ResolutionTrace {
    fn on_event(&mut self, event: SolverEvent<'_, String, Ranges<Version>, String>) {
        match event {
            SolverEvent::PackageChoice { package, allowed } => {
                if let Some(watched) = self.watched.get_mut(package)
                    && allowed.contains(&watched.version)
                {
                    // A previous exclusion may have been undone by backtracking.
                    watched.decision_level = None;
                    watched.reason = None;
                }
            }
            SolverEvent::VersionChoice {
                package,
                version,
                allowed,
            } => {
                if let Some(watched) = self.watched.get_mut(package) {
                    if version == &watched.version {
                        watched.reason = None;
                    } else if allowed.contains(&watched.version) {
                        watched.reason =
                            Some(SkippedVersionReason::ProviderPreferred(version.clone()));
                    }
                }
            }
            SolverEvent::Decision {
                package,
                version,
                decision_level,
            } => {
                if let Some(watched) = self.watched.get_mut(package)
                    && version == &watched.version
                {
                    watched.decision_level = Some(decision_level);
                    watched.reason = None;
                }
            }
            SolverEvent::Derivation {
                package,
                previous,
                current,
                cause,
            } => {
                if let Some(watched) = self.watched.get_mut(package) {
                    let was_allowed = previous.is_none_or(|term| term.contains(&watched.version));
                    if was_allowed && !current.contains(&watched.version) {
                        watched.decision_level = None;
                        if !matches!(watched.reason, Some(SkippedVersionReason::Backtracked(_))) {
                            watched.reason =
                                Some(SkippedVersionReason::ExcludedByPropagation(cause.clone()));
                        }
                    }
                }
            }
            SolverEvent::Backtrack {
                from_level,
                to_level,
                cause,
            } => {
                for watched in self.watched.values_mut() {
                    if watched
                        .decision_level
                        .is_some_and(|level| level > to_level && level <= from_level)
                    {
                        watched.decision_level = None;
                        watched.reason = Some(SkippedVersionReason::Backtracked(cause.clone()));
                    }
                }
            }
            SolverEvent::NoVersion { .. }
            | SolverEvent::Conflict { .. }
            | SolverEvent::Solution => {}
            _ => {}
        }
    }
}

fn external_facts(cause: &Cause) -> Vec<String> {
    let mut facts = Vec::new();
    collect_external_facts(cause, &mut facts);
    facts
}

fn collect_external_facts(cause: &Cause, facts: &mut Vec<String>) {
    match cause {
        DerivationTree::External(external) => {
            let fact = match external {
                External::NotRoot(package, version) => {
                    format!(
                        "{} {version} is not the project root",
                        package_name(package)
                    )
                }
                External::NoVersions(package, versions) => format!(
                    "no available version of {} matches {versions}",
                    package_name(package)
                ),
                External::FromDependencyOf(package, versions, dependency, required) => {
                    if is_internal_root(package) {
                        format!(
                            "the project requires {} {required}",
                            package_name(dependency)
                        )
                    } else {
                        format!(
                            "{} {versions} requires {} {required}",
                            package_name(package),
                            package_name(dependency)
                        )
                    }
                }
                External::Custom(package, versions, message) => format!(
                    "{} {versions} is unavailable: {message}",
                    package_name(package)
                ),
            };
            if !facts.contains(&fact) {
                facts.push(fact);
            }
        }
        DerivationTree::Derived(derived) => {
            collect_external_facts(&derived.cause1, facts);
            collect_external_facts(&derived.cause2, facts);
        }
    }
}

fn is_internal_root(package: &str) -> bool {
    package.starts_with("___") && package.ends_with("___")
}

fn package_name(package: &str) -> &str {
    if is_internal_root(package) {
        "the project"
    } else {
        package
    }
}

#[cfg(test)]
mod tests {
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
}
