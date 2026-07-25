//! Converts typed solver events into explanations for skipped upgrade candidates.

mod render;
#[cfg(test)]
mod tests;

use std::collections::HashMap;

use pubgrub::{DerivationTree, MaximalityProbeResult, Ranges, SolverEvent, SolverObserver};

use crate::progress::{
    ProgressEvent, ProgressReporter, ResolutionActivity, ResolutionWork, emit as emit_progress,
};
use crate::resolver::types::{CandidateDiagnostic, SolverPackage, SolverVersion};

pub(super) type Cause = DerivationTree<SolverPackage, Ranges<SolverVersion>, String>;

#[derive(Debug, Clone)]
pub(super) enum SkippedVersionReason {
    ExcludedByPropagation(Cause),
    Backtracked(Cause),
}

#[derive(Debug, Clone)]
pub(super) struct WatchedVersion {
    pub(super) version: SolverVersion,
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
    progress: Option<ProgressReporter>,
    probe_checkpoint: Option<HashMap<SolverPackage, WatchedVersion>>,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolutionSnapshot {
    watched: HashMap<SolverPackage, WatchedVersion>,
}

impl ResolutionTrace {
    pub(crate) fn with_progress(
        candidates: impl IntoIterator<Item = (String, SolverVersion)>,
        progress: Option<ProgressReporter>,
    ) -> Self {
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
            progress,
            probe_checkpoint: None,
        }
    }

    pub(crate) fn into_solutions(self) -> Vec<ResolutionSnapshot> {
        self.solutions
    }

    fn report_progress(
        &self,
        event: &SolverEvent<'_, SolverPackage, Ranges<SolverVersion>, String>,
    ) {
        let event = match event {
            SolverEvent::EnumerationRunStarted { run } => ProgressEvent::ResolutionWorkStarted {
                work: ResolutionWork::EnumerationRun { run: *run },
            },
            SolverEvent::EnumerationRunFinished { run } => ProgressEvent::ResolutionWorkFinished {
                work: ResolutionWork::EnumerationRun { run: *run },
            },
            SolverEvent::MaximalityProbeStarted { package } => {
                ProgressEvent::ResolutionWorkStarted {
                    work: ResolutionWork::MaximalityProbe {
                        package: package.to_string(),
                    },
                }
            }
            SolverEvent::MaximalityProbeFinished { package, .. } => {
                ProgressEvent::ResolutionWorkFinished {
                    work: ResolutionWork::MaximalityProbe {
                        package: package.to_string(),
                    },
                }
            }
            SolverEvent::Decision { package, .. } => ProgressEvent::ResolutionActivity {
                activity: ResolutionActivity::Decision {
                    package: package.to_string(),
                },
            },
            SolverEvent::Derivation { package, .. } => ProgressEvent::ResolutionActivity {
                activity: ResolutionActivity::Propagation {
                    package: package.to_string(),
                },
            },
            SolverEvent::Backtrack {
                from_level,
                to_level,
                ..
            } => ProgressEvent::ResolutionActivity {
                activity: ResolutionActivity::Backtrack {
                    from_level: *from_level,
                    to_level: *to_level,
                },
            },
            SolverEvent::Conflict { .. } => ProgressEvent::ResolutionActivity {
                activity: ResolutionActivity::Conflict,
            },
            SolverEvent::Solution => ProgressEvent::ResolutionActivity {
                activity: ResolutionActivity::Solution,
            },
            _ => return,
        };
        emit_progress(self.progress.as_ref(), event);
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
        self.report_progress(&event);
        match event {
            SolverEvent::MaximalityProbeStarted { .. } => {
                assert!(
                    self.probe_checkpoint.is_none(),
                    "maximality probes must not be nested"
                );
                self.probe_checkpoint = Some(self.watched.clone());
                for watched in self.watched.values_mut() {
                    watched.decision_level = None;
                    watched.reason = None;
                }
            }
            SolverEvent::MaximalityProbeFinished { result, .. } => {
                let checkpoint = self
                    .probe_checkpoint
                    .take()
                    .expect("a maximality probe finish must match a start");
                if result != MaximalityProbeResult::Improved {
                    self.watched = checkpoint;
                }
            }
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
                allowed: _,
            } => {
                if let Some(watched) = self.watched.get_mut(package)
                    && version == &watched.version
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
