use pubgrub::{DerivationTree, External};

use super::{Cause, SkippedVersionReason, WatchedVersion};
use crate::resolver::types::{CandidateDiagnostic, CandidateDiagnosticKind};
use crate::versions::Version;

pub(super) fn diagnose(
    package: &str,
    selected: &Version,
    watched: Option<&WatchedVersion>,
) -> CandidateDiagnostic {
    let Some(watched) = watched else {
        return CandidateDiagnostic {
            package: package.to_string(),
            selected_version: selected.to_string(),
            candidate_version: "?".to_string(),
            kind: CandidateDiagnosticKind::Unexplained,
            preferred_version: None,
            facts: vec!["no candidate trace was recorded".to_string()],
        };
    };

    let (kind, preferred_version, facts) = match &watched.reason {
        Some(SkippedVersionReason::ExcludedByPropagation(cause)) => (
            CandidateDiagnosticKind::ExcludedByPropagation,
            None,
            facts_for_cause(cause),
        ),
        Some(SkippedVersionReason::Backtracked(cause)) => (
            CandidateDiagnosticKind::Backtracked,
            None,
            facts_for_cause(cause),
        ),
        Some(SkippedVersionReason::ProviderPreferred(preferred)) => (
            CandidateDiagnosticKind::ProviderPreferred,
            Some(preferred.to_string()),
            Vec::new(),
        ),
        None => (CandidateDiagnosticKind::Unexplained, None, Vec::new()),
    };

    CandidateDiagnostic {
        package: package.to_string(),
        selected_version: selected.to_string(),
        candidate_version: watched.version.to_string(),
        kind,
        preferred_version,
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
        let mut packages: Vec<_> = cause.packages().into_iter().cloned().collect();
        packages.sort();
        facts.push(format!("conflict involved {}", packages.join(", ")));
    }

    const MAX_FACTS: usize = 8;
    if facts.len() > MAX_FACTS {
        let remaining = facts.len() - MAX_FACTS;
        facts.truncate(MAX_FACTS);
        facts.push(format!("and {remaining} more dependency fact(s)"));
    }
    facts
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
                External::CustomClause { metadata, .. } => metadata.clone(),
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
    } else if let Some(capability) = package.strip_prefix("___orbit_capability___") {
        capability
    } else if let Some(choice) = package.strip_prefix("___orbit_provider_choice___") {
        choice.split("___").next().unwrap_or(choice)
    } else if let Some(artifact) = package.strip_prefix("___orbit_jarjar___") {
        artifact
    } else {
        package
    }
}
