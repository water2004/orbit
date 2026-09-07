use pubgrub::{DerivationTree, External};
use std::ops::Bound;

use super::{Cause, SkippedVersionReason, WatchedVersion};
use crate::resolver::types::{CandidateDiagnostic, CandidateDiagnosticKind};
use crate::resolver::types::{SolverPackage, SolverVersion};
use crate::versions::Version;

pub(super) fn diagnose(
    package: &str,
    selected: &SolverVersion,
    watched: Option<&WatchedVersion>,
) -> CandidateDiagnostic {
    let Some(watched) = watched else {
        return CandidateDiagnostic {
            package: package.to_string(),
            selected_version: selected.to_string(),
            candidate_version: "?".to_string(),
            kind: CandidateDiagnosticKind::Unexplained,
            facts: vec!["no candidate trace was recorded".to_string()],
        };
    };

    let (kind, facts) = match &watched.reason {
        Some(SkippedVersionReason::ExcludedByPropagation(cause)) => (
            CandidateDiagnosticKind::ExcludedByPropagation,
            facts_for_cause(cause),
        ),
        Some(SkippedVersionReason::Backtracked(cause)) => {
            (CandidateDiagnosticKind::Backtracked, facts_for_cause(cause))
        }
        None => (CandidateDiagnosticKind::Unexplained, Vec::new()),
    };

    CandidateDiagnostic {
        package: package.to_string(),
        selected_version: selected.to_string(),
        candidate_version: watched.version.to_string(),
        kind,
        facts,
    }
}

pub(super) fn describe_no_solution(cause: &Cause) -> String {
    let facts = facts_for_cause(cause);
    let mut output = "dependency resolution failed".to_string();
    for fact in facts {
        output.push_str("\n  - ");
        output.push_str(&fact);
    }
    output
}

fn facts_for_cause(cause: &Cause) -> Vec<String> {
    let mut facts = external_facts(cause);
    if facts.is_empty() {
        let mut packages: Vec<_> = cause
            .packages()
            .into_iter()
            .filter(|package| !matches!(package, SolverPackage::LoadPreference { .. }))
            .map(|package| package.user_label().to_string())
            .collect();
        packages.sort();
        packages.dedup();
        if packages.is_empty() {
            facts.push("the selected versions are mutually incompatible".to_string());
        } else {
            facts.push(format!("conflict involved {}", packages.join(", ")));
        }
    }

    const MAX_FACTS: usize = 8;
    if facts.len() > MAX_FACTS {
        let remaining = facts.len() - MAX_FACTS;
        facts.truncate(MAX_FACTS);
        facts.push(format!("and {remaining} more dependency fact(s)"));
    }
    facts
}

pub(super) fn has_domain_facts(cause: &Cause) -> bool {
    !external_facts(cause).is_empty()
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
                External::NotRoot(package, version) => Some(format!(
                    "{} {version} is not the project root",
                    package.user_label()
                )),
                External::NoVersions(SolverPackage::LoadPreference { .. }, _) => None,
                External::NoVersions(_, versions) if is_excluded_semantic_singleton(versions) => {
                    None
                }
                External::NoVersions(package, versions) => Some(format!(
                    "no available version of {} matches {}",
                    package.user_label(),
                    display_versions(versions)
                )),
                External::FromDependencyOf(package, _, dependency, _)
                    if matches!(package, SolverPackage::LoadPreference { .. })
                        || matches!(dependency, SolverPackage::LoadPreference { .. })
                        || package == dependency =>
                {
                    None
                }
                External::FromDependencyOf(package, versions, dependency, required) => {
                    if package == &SolverPackage::Root {
                        Some(format!(
                            "the project requires {} {}",
                            dependency.user_label(),
                            display_unique(required)
                        ))
                    } else {
                        Some(format!(
                            "{} {} requires {} {}",
                            package.user_label(),
                            display_unique(versions),
                            dependency.user_label(),
                            display_unique(required)
                        ))
                    }
                }
                External::Custom(package, versions, message) => Some(format!(
                    "{} {versions} is unavailable: {message}",
                    package.user_label()
                )),
                External::CustomClause { metadata, .. } => Some(metadata.clone()),
                External::ExcludedSolution { .. } => return,
            };
            if let Some(fact) = fact
                && !facts.contains(&fact)
            {
                facts.push(fact);
            }
        }
        DerivationTree::Derived(derived) => {
            collect_external_facts(&derived.cause1, facts);
            collect_external_facts(&derived.cause2, facts);
        }
    }
}

fn display_versions(versions: &pubgrub::Ranges<SolverVersion>) -> String {
    fabric_wildcard(versions).unwrap_or_else(|| display_unique(versions))
}

/// Several candidate identities can share one declared version; their
/// singleton segments render identically, so collapse duplicates for display.
fn display_unique(versions: &pubgrub::Ranges<SolverVersion>) -> String {
    let mut seen = std::collections::HashSet::new();
    let mut segments = Vec::new();
    for segment in versions.clone() {
        let rendered = std::iter::once(segment)
            .collect::<pubgrub::Ranges<SolverVersion>>()
            .to_string();
        if seen.insert(rendered.clone()) {
            segments.push(rendered);
        }
    }
    if segments.is_empty() {
        return versions.to_string();
    }
    segments.join(" | ")
}

fn fabric_wildcard(versions: &pubgrub::Ranges<SolverVersion>) -> Option<String> {
    let intervals: Vec<_> = versions.clone().into_iter().collect();
    let [(Bound::Included(lower), Bound::Excluded(upper))] = intervals.as_slice() else {
        return None;
    };
    let (Some(Version::Fabric(lower)), Some(Version::Fabric(upper))) =
        (lower.domain(), upper.domain())
    else {
        return None;
    };
    crate::versions::fabric::wildcard_for_core_bounds(lower, upper)
}

fn is_excluded_semantic_singleton(versions: &pubgrub::Ranges<SolverVersion>) -> bool {
    let intervals: Vec<_> = versions.clone().into_iter().collect();
    let [
        (Bound::Unbounded, Bound::Excluded(lower_edge)),
        (Bound::Excluded(upper_edge), Bound::Unbounded),
    ] = intervals.as_slice()
    else {
        return false;
    };
    lower_edge.domain().is_some() && lower_edge.domain() == upper_edge.domain()
}

#[cfg(test)]
mod rendering_tests {
    use super::*;
    use crate::loader::LoaderKind;
    use crate::resolver::types::solver_range;

    #[test]
    fn renders_fabric_wildcard_without_synthetic_bounds() {
        let versions = solver_range(Version::parse_constraint("0.9.x", LoaderKind::Fabric));

        assert_eq!(display_versions(&versions), "0.9.x");
    }

    #[test]
    fn collapses_candidate_singletons_sharing_one_declared_version() {
        use crate::resolver::types::{CandidateIdentity, CandidateLocation};

        let identity = |source: &str, installed: bool| CandidateIdentity {
            owner: "a".to_string(),
            source: source.to_string(),
            path: Vec::new(),
            location: CandidateLocation::Root,
            installed,
        };
        let semantic = || Version::parse("1.0.0", LoaderKind::Fabric);
        let versions = pubgrub::Ranges::singleton(SolverVersion::candidate(
            semantic(),
            identity("modrinth:abc", true),
        ))
        .union(&pubgrub::Ranges::singleton(SolverVersion::candidate(
            semantic(),
            identity("file:sources/a.jar", false),
        )));

        assert_eq!(display_unique(&versions), "1.0.0");
    }

    #[test]
    fn recognizes_internal_exclusion_of_one_semantic_version() {
        let versions = solver_range(pubgrub::Ranges::singleton(Version::parse(
            "1.11.2+mc26.1.2",
            LoaderKind::Fabric,
        )))
        .complement();

        assert!(is_excluded_semantic_singleton(&versions));
    }
}
