use pubgrub::{DerivationTree, External};

use super::{Cause, SkippedVersionReason, WatchedVersion};
use crate::versions::Version;

pub(super) fn describe(
    package: &str,
    selected: &Version,
    watched: Option<&WatchedVersion>,
) -> String {
    let Some(watched) = watched else {
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
    append_facts(&mut output, cause);
    output
}

fn append_facts(output: &mut String, cause: &Cause) {
    let facts = external_facts(cause);
    if facts.is_empty() {
        let mut packages: Vec<_> = cause.packages().into_iter().cloned().collect();
        packages.sort();
        output.push_str(&format!(
            "\n      - conflict involved {}",
            packages.join(", ")
        ));
        return;
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
