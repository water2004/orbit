//! Converts typed solver events into explanations for skipped upgrade candidates.

mod render;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use pubgrub::{DerivationTree, MaximalityProbeResult, Ranges, SolverEvent, SolverObserver};

use crate::progress::{ProgressEvent, ProgressReporter, ResolutionCurrent, emit as emit_progress};
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
    progress_state: ResolutionProgress,
    pending_progress_events: u64,
    last_progress_emit: Instant,
    probe_checkpoint: Option<HashMap<SolverPackage, WatchedVersion>>,
}

#[derive(Default)]
struct ResolutionProgress {
    work_discovered: u64,
    work_completed: u64,
    decisions: u64,
    propagations: u64,
    backtracks: u64,
    conflicts: u64,
    solutions: usize,
    current: Option<ResolutionCurrent>,
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
            progress_state: ResolutionProgress::default(),
            pending_progress_events: 0,
            last_progress_emit: Instant::now(),
            probe_checkpoint: None,
        }
    }

    pub(crate) fn into_solutions(mut self) -> Vec<ResolutionSnapshot> {
        self.flush_progress();
        self.solutions
    }

    pub(crate) fn flush_progress(&mut self) {
        if self.pending_progress_events == 0 {
            return;
        }
        emit_progress(
            self.progress.as_ref(),
            ProgressEvent::ResolutionAdvanced {
                work_discovered: self.progress_state.work_discovered,
                work_completed: self.progress_state.work_completed,
                decisions: self.progress_state.decisions,
                propagations: self.progress_state.propagations,
                backtracks: self.progress_state.backtracks,
                conflicts: self.progress_state.conflicts,
                solutions: self.progress_state.solutions,
                current: self.progress_state.current.clone(),
            },
        );
        self.pending_progress_events = 0;
        self.last_progress_emit = Instant::now();
    }

    fn report_progress(
        &mut self,
        event: &SolverEvent<'_, SolverPackage, Ranges<SolverVersion>, String>,
    ) {
        match event {
            SolverEvent::EnumerationRunStarted { run } => {
                self.progress_state.work_discovered += 1;
                self.progress_state.current = Some(ResolutionCurrent::Enumeration { run: *run });
            }
            SolverEvent::EnumerationRunFinished { .. } => {
                self.progress_state.work_completed += 1;
            }
            SolverEvent::MaximalityProbeStarted { package } => {
                self.progress_state.work_discovered += 1;
                self.progress_state.current = Some(ResolutionCurrent::VersionMaximization {
                    package: package.to_string(),
                });
            }
            SolverEvent::MaximalityProbeFinished { .. } => {
                self.progress_state.work_completed += 1;
            }
            SolverEvent::PreferenceProbeStarted { package } => {
                self.progress_state.work_discovered += 1;
                self.progress_state.current = Some(ResolutionCurrent::PreferencePreservation {
                    package: package.to_string(),
                });
            }
            SolverEvent::PreferenceProbeFinished { .. } => {
                self.progress_state.work_completed += 1;
            }
            SolverEvent::Decision { package, .. } => {
                self.progress_state.decisions += 1;
                self.progress_state.current = Some(ResolutionCurrent::Decision {
                    package: package.to_string(),
                });
            }
            SolverEvent::Derivation { .. } => self.progress_state.propagations += 1,
            SolverEvent::Backtrack { .. } => self.progress_state.backtracks += 1,
            SolverEvent::Conflict { .. } => self.progress_state.conflicts += 1,
            SolverEvent::Solution => self.progress_state.solutions += 1,
            _ => return,
        }
        self.pending_progress_events += 1;
        if self.pending_progress_events >= 512
            || self.last_progress_emit.elapsed() >= Duration::from_millis(100)
            || matches!(event, SolverEvent::Solution)
        {
            self.flush_progress();
        }
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
