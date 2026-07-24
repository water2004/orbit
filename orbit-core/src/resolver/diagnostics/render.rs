use pubgrub::{DerivationTree, External};

use super::{Cause, SkippedVersionReason, WatchedVersion};
use crate::resolver::types::{CandidateDiagnostic, CandidateDiagnosticKind};
use crate::resolver::types::{SolverPackage, SolverVersion};

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
            .filter(|package| !matches!(package, SolverPackage::ProviderChoice { .. }))
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
                External::NoVersions(
                    SolverPackage::Mod(_)
                    | SolverPackage::Bundled { .. }
                    | SolverPackage::ProviderChoice { .. },
                    _,
                ) => None,
                External::NoVersions(package, versions) => Some(format!(
                    "no available version of {} matches {versions}",
                    package.user_label()
                )),
                External::FromDependencyOf(package, _, dependency, _)
                    if matches!(package, SolverPackage::ProviderChoice { .. })
                        || matches!(dependency, SolverPackage::ProviderChoice { .. })
                        || matches!(
                            (package, dependency),
                            (
                                SolverPackage::Mod(_) | SolverPackage::Bundled { .. },
                                SolverPackage::Mod(_) | SolverPackage::Bundled { .. }
                            )
                        ) =>
                {
                    None
                }
                External::FromDependencyOf(package, versions, dependency, required) => {
                    if package == &SolverPackage::Root {
                        Some(format!(
                            "the project requires {} {required}",
                            dependency.user_label()
                        ))
                    } else {
                        Some(format!(
                            "{} {versions} requires {} {required}",
                            package.user_label(),
                            dependency.user_label()
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
