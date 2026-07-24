//! Converts typed solver events into explanations for skipped upgrade candidates.

mod render;
#[cfg(test)]
mod tests;

use std::collections::HashMap;

use pubgrub::{DerivationTree, Ranges, SolverEvent, SolverObserver};

use crate::resolver::types::CandidateDiagnostic;
use crate::versions::Version;

pub(super) type Cause = DerivationTree<String, Ranges<Version>, String>;

#[derive(Debug, Clone)]
pub(super) enum SkippedVersionReason {
    ExcludedByPropagation(Cause),
    Backtracked(Cause),
    ProviderPreferred(Version),
}

#[derive(Debug)]
pub(super) struct WatchedVersion {
    pub(super) version: Version,
    decision_level: Option<u32>,
    pub(super) reason: Option<SkippedVersionReason>,
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

    pub(crate) fn diagnose_skipped(
        &self,
        package: &str,
        selected: &Version,
    ) -> CandidateDiagnostic {
        render::diagnose(package, selected, self.watched.get(package))
    }
}

pub(crate) fn describe_no_solution(cause: &Cause) -> String {
    render::describe_no_solution(cause)
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
