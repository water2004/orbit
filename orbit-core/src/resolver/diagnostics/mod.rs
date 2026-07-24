//! Converts typed solver events into explanations for skipped upgrade candidates.

mod render;
#[cfg(test)]
mod tests;

use std::collections::HashMap;

use pubgrub::{DerivationTree, Ranges, SolverEvent, SolverObserver};

use crate::resolver::types::{CandidateDiagnostic, SolverPackage, SolverVersion};
use crate::versions::Version;

pub(super) type Cause = DerivationTree<SolverPackage, Ranges<SolverVersion>, String>;

#[derive(Debug, Clone)]
pub(super) enum SkippedVersionReason {
    ExcludedByPropagation(Cause),
    Backtracked(Cause),
}

#[derive(Debug, Clone)]
pub(super) struct WatchedVersion {
    pub(super) version: Version,
    decision_level: Option<u32>,
    pub(super) reason: Option<SkippedVersionReason>,
}

impl WatchedVersion {
    fn record_reason(&mut self, reason: SkippedVersionReason) {
        let new_is_domain = render::has_domain_facts(reason.cause());
        let current_is_domain = self
            .reason
            .as_ref()
            .is_some_and(|current| render::has_domain_facts(current.cause()));
        if new_is_domain || !current_is_domain {
            self.reason = Some(reason);
        }
    }

    fn record_backtrack(&mut self, cause: &Cause) {
        let cause = if render::has_domain_facts(cause) {
            cause.clone()
        } else if let Some(current) = &self.reason
            && render::has_domain_facts(current.cause())
        {
            current.cause().clone()
        } else {
            cause.clone()
        };
        self.reason = Some(SkippedVersionReason::Backtracked(cause));
    }
}

impl SkippedVersionReason {
    fn cause(&self) -> &Cause {
        match self {
            Self::ExcludedByPropagation(cause) | Self::Backtracked(cause) => cause,
        }
    }
}

/// Records why candidate versions were skipped during the successful solver run.
pub(crate) struct ResolutionTrace {
    watched: HashMap<SolverPackage, WatchedVersion>,
    solutions: Vec<ResolutionSnapshot>,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolutionSnapshot {
    watched: HashMap<SolverPackage, WatchedVersion>,
}

impl ResolutionTrace {
    pub(crate) fn new(candidates: impl IntoIterator<Item = (String, Version)>) -> Self {
        Self {
            watched: candidates
                .into_iter()
                .map(|(package, version)| {
                    (
                        SolverPackage::logical(package),
                        WatchedVersion {
                            version,
                            decision_level: None,
                            reason: None,
                        },
                    )
                })
                .collect(),
            solutions: Vec::new(),
        }
    }

    pub(crate) fn into_solutions(self) -> Vec<ResolutionSnapshot> {
        self.solutions
    }
}

impl ResolutionSnapshot {
    pub(crate) fn diagnose_skipped(
        &self,
        package: &str,
        selected: &SolverVersion,
    ) -> CandidateDiagnostic {
        render::diagnose(
            package,
            selected,
            self.watched.get(&SolverPackage::logical(package)),
        )
    }
}

pub(crate) fn describe_no_solution(cause: &Cause) -> String {
    render::describe_no_solution(cause)
}

impl SolverObserver<SolverPackage, Ranges<SolverVersion>, String> for ResolutionTrace {
    fn on_event(&mut self, event: SolverEvent<'_, SolverPackage, Ranges<SolverVersion>, String>) {
        match event {
            SolverEvent::PackageChoice { package, allowed } => {
                if let Some(watched) = self.watched.get_mut(package)
                    && allowed.contains(&SolverVersion::Domain(watched.version.clone()))
                {
                    // A previous exclusion may have been undone by backtracking.
                    watched.decision_level = None;
                    watched.reason = None;
                }
            }
            SolverEvent::VersionChoice {
                package,
                version,
                allowed: _,
            } => {
                if let Some(watched) = self.watched.get_mut(package)
                    && version.domain() == Some(&watched.version)
                {
                    watched.reason = None;
                }
            }
            SolverEvent::Decision {
                package,
                version,
                decision_level,
            } => {
                if let Some(watched) = self.watched.get_mut(package)
                    && version.domain() == Some(&watched.version)
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
                    let watched_version = SolverVersion::Domain(watched.version.clone());
                    let was_allowed = previous.is_none_or(|term| term.contains(&watched_version));
                    if was_allowed && !current.contains(&watched_version) {
                        watched.decision_level = None;
                        if !matches!(watched.reason, Some(SkippedVersionReason::Backtracked(_)))
                            || render::has_domain_facts(cause)
                        {
                            watched.record_reason(SkippedVersionReason::ExcludedByPropagation(
                                cause.clone(),
                            ));
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
                        watched.record_backtrack(cause);
                    }
                }
            }
            SolverEvent::NoVersion { .. } | SolverEvent::Conflict { .. } => {}
            SolverEvent::Solution => self.solutions.push(ResolutionSnapshot {
                watched: self.watched.clone(),
            }),
            _ => {}
        }
    }
}
