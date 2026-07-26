use std::io::IsTerminal;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use orbit_core::{
    ArtifactProgressState, AuditProgressEvent, AuditProgressReporter, AuditProgressStage,
    ProgressEvent, ProgressReporter, ResolutionActivity, ResolutionWork,
};

pub fn reporter(quiet: bool, configured_style: &str) -> Option<ProgressReporter> {
    if quiet
        || matches!(
            configured_style.trim().to_ascii_lowercase().as_str(),
            "off" | "none" | "false"
        )
    {
        return None;
    }

    let modern =
        configured_style.trim().eq_ignore_ascii_case("modern") && std::io::stderr().is_terminal();
    let renderer = Arc::new(ProgressRenderer {
        modern,
        state: Mutex::new(RenderState::default()),
    });
    Some(Arc::new(move |event| renderer.render(event)))
}

pub fn audit_reporter(quiet: bool, configured_style: &str) -> Option<AuditProgressReporter> {
    if quiet
        || matches!(
            configured_style.trim().to_ascii_lowercase().as_str(),
            "off" | "none" | "false"
        )
    {
        return None;
    }

    let modern =
        configured_style.trim().eq_ignore_ascii_case("modern") && std::io::stderr().is_terminal();
    let renderer = Arc::new(AuditProgressRenderer {
        modern,
        state: Mutex::new(RenderState::default()),
    });
    Some(Arc::new(move |event| renderer.render(event)))
}

struct AuditProgressRenderer {
    modern: bool,
    state: Mutex<RenderState>,
}

impl AuditProgressRenderer {
    fn render(&self, event: AuditProgressEvent) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if !self.modern {
            if let Some(line) = audit_plain_line(&event) {
                eprintln!("{line}");
            }
            return;
        }

        match event {
            AuditProgressEvent::StageStarted { stage, total } => {
                let message = format!(
                    "[{}/6] {}",
                    audit_stage_number(stage),
                    audit_stage_present(stage)
                );
                if let Some(total) = total.filter(|total| *total > 0) {
                    start_bar(&mut state, total, message);
                } else {
                    start_spinner(&mut state, message);
                }
            }
            AuditProgressEvent::Advanced {
                stage,
                completed,
                total,
            } => {
                if let Some(bar) = &state.bar {
                    if let Some(total) = total {
                        bar.set_length(total as u64);
                    }
                    bar.set_position(completed as u64);
                    bar.set_message(format!(
                        "[{}/6] {}",
                        audit_stage_number(stage),
                        audit_stage_present(stage)
                    ));
                }
            }
            AuditProgressEvent::StageFinished { stage, completed } => finish(
                &mut state,
                format!(
                    "[{}/6] {}",
                    audit_stage_number(stage),
                    audit_stage_finished(stage, completed)
                ),
            ),
        }
    }
}

impl Drop for AuditProgressRenderer {
    fn drop(&mut self) {
        if let Ok(state) = self.state.get_mut()
            && let Some(bar) = state.bar.take()
        {
            bar.finish_and_clear();
        }
    }
}

struct ProgressRenderer {
    modern: bool,
    state: Mutex<RenderState>,
}

#[derive(Default)]
struct RenderState {
    bar: Option<ProgressBar>,
    resolution_total: usize,
    resolution_completed: usize,
    decisions: usize,
    propagations: usize,
    backtracks: usize,
    conflicts: usize,
    solutions: usize,
}

impl ProgressRenderer {
    fn render(&self, event: ProgressEvent) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if !self.modern {
            if let Some(line) = plain_line(&event, &mut state) {
                eprintln!("{line}");
            }
            return;
        }

        match event {
            ProgressEvent::DiscoveryStarted => start_spinner(
                &mut state,
                "[1/4] Discovering provider projects and candidate versions",
            ),
            ProgressEvent::DiscoveringProject {
                provider,
                locator,
                pending_projects,
                artifacts_found,
            } => set_message(
                &state,
                format!(
                    "[1/4] {provider}: {locator} ({pending_projects} pending, {artifacts_found} JARs found)"
                ),
            ),
            ProgressEvent::DiscoveryFinished {
                projects,
                artifacts,
            } => finish(
                &mut state,
                format!("[1/4] Found {artifacts} candidate JARs in {projects} projects"),
            ),
            ProgressEvent::CandidateDownloadStarted { total } => start_bar(
                &mut state,
                total,
                format!("[2/4] Downloading, verifying, and parsing {total} candidate JARs"),
            ),
            ProgressEvent::CandidateArtifact {
                completed,
                total,
                filename: _,
                state: artifact_state,
            } => {
                if let Some(bar) = &state.bar {
                    bar.set_length(total as u64);
                    bar.set_position(completed as u64);
                    let action = match artifact_state {
                        ArtifactProgressState::Started => "processing",
                        ArtifactProgressState::Finished => "parsed",
                        ArtifactProgressState::AlreadyPresent => "cached",
                        ArtifactProgressState::Failed => "failed",
                    };
                    bar.set_message(format!("[2/4] {action} candidate {completed}/{total}"));
                }
            }
            ProgressEvent::CandidateDownloadFinished { total } => finish(
                &mut state,
                format!("[2/4] Downloaded/verified and parsed {total} candidate JARs"),
            ),
            ProgressEvent::ResolutionStarted {
                packages,
                candidates,
            } => {
                state.reset_resolution();
                start_bar(
                    &mut state,
                    0,
                    format!(
                        "[3/4] Resolving {packages} packages across {candidates} JAR candidates"
                    ),
                );
            }
            ProgressEvent::ResolutionWorkStarted { work } => {
                state.resolution_total += 1;
                update_resolution_bar(
                    &state,
                    format!(
                        "[3/4] {} · {}",
                        work_started_label(&work),
                        state.resolution_counters()
                    ),
                );
            }
            ProgressEvent::ResolutionWorkFinished { work } => {
                state.resolution_completed += 1;
                update_resolution_bar(
                    &state,
                    format!(
                        "[3/4] {} · {}",
                        work_finished_label(&work),
                        state.resolution_counters()
                    ),
                );
            }
            ProgressEvent::ResolutionActivity { activity } => {
                state.record_activity(&activity);
                set_message(
                    &state,
                    format!(
                        "[3/4] {} · {}",
                        activity_label(&activity),
                        state.resolution_counters()
                    ),
                );
            }
            ProgressEvent::ResolutionFinished { solutions } => {
                state.solutions = solutions;
                let counters = state.resolution_counters();
                finish(
                    &mut state,
                    format!("[3/4] Found {solutions} Pareto-maximal solution(s) · {counters}"),
                );
            }
            ProgressEvent::ApplyStarted { total } => start_bar(
                &mut state,
                total,
                format!("[4/4] Applying {total} selected packages"),
            ),
            ProgressEvent::ApplyArtifact {
                completed,
                total,
                filename: _,
                state: artifact_state,
            } => {
                if let Some(bar) = &state.bar {
                    bar.set_length(total as u64);
                    bar.set_position(completed as u64);
                    let action = match artifact_state {
                        ArtifactProgressState::Started => "applying",
                        ArtifactProgressState::Finished => "installed",
                        ArtifactProgressState::AlreadyPresent => "already present",
                        ArtifactProgressState::Failed => "failed",
                    };
                    bar.set_message(format!("[4/4] {action} package {completed}/{total}"));
                }
            }
            ProgressEvent::ApplyFinished { total } => finish(
                &mut state,
                format!("[4/4] Applied/verified {total} selected packages"),
            ),
        }
    }
}

impl Drop for ProgressRenderer {
    fn drop(&mut self) {
        if let Ok(state) = self.state.get_mut()
            && let Some(bar) = state.bar.take()
        {
            bar.finish_and_clear();
        }
    }
}

impl RenderState {
    fn reset_resolution(&mut self) {
        self.resolution_total = 0;
        self.resolution_completed = 0;
        self.decisions = 0;
        self.propagations = 0;
        self.backtracks = 0;
        self.conflicts = 0;
        self.solutions = 0;
    }

    fn record_activity(&mut self, activity: &ResolutionActivity) {
        match activity {
            ResolutionActivity::Decision { .. } => self.decisions += 1,
            ResolutionActivity::Propagation { .. } => self.propagations += 1,
            ResolutionActivity::Backtrack { .. } => self.backtracks += 1,
            ResolutionActivity::Conflict => self.conflicts += 1,
            ResolutionActivity::Solution => self.solutions += 1,
        }
    }

    fn resolution_counters(&self) -> String {
        format!(
            "work {}/{}, {} decisions, {} propagations, {} backtracks, {} conflicts, {} solutions",
            self.resolution_completed,
            self.resolution_total,
            self.decisions,
            self.propagations,
            self.backtracks,
            self.conflicts,
            self.solutions
        )
    }
}

fn start_spinner(state: &mut RenderState, message: impl Into<String>) {
    clear(state);
    let bar = ProgressBar::with_draw_target(None, ProgressDrawTarget::stderr_with_hz(10));
    bar.set_style(
        ProgressStyle::with_template("{spinner:.cyan} [{elapsed_precise}] {msg}")
            .expect("static progress template must be valid"),
    );
    bar.set_message(message.into());
    bar.enable_steady_tick(Duration::from_millis(80));
    state.bar = Some(bar);
}

fn start_bar(state: &mut RenderState, total: usize, message: impl Into<String>) {
    clear(state);
    let bar =
        ProgressBar::with_draw_target(Some(total as u64), ProgressDrawTarget::stderr_with_hz(10));
    bar.set_style(
        ProgressStyle::with_template(
            "[{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos:>4}/{len:4} {msg}",
        )
        .expect("static progress template must be valid")
        .progress_chars("=>-"),
    );
    bar.set_message(message.into());
    state.bar = Some(bar);
}

fn set_message(state: &RenderState, message: String) {
    if let Some(bar) = &state.bar {
        bar.set_message(message);
    }
}

fn update_resolution_bar(state: &RenderState, message: String) {
    if let Some(bar) = &state.bar {
        bar.set_length(state.resolution_total as u64);
        bar.set_position(state.resolution_completed as u64);
        bar.set_message(message);
    }
}

fn finish(state: &mut RenderState, message: String) {
    if let Some(bar) = state.bar.take() {
        bar.finish_with_message(message);
    }
}

fn clear(state: &mut RenderState) {
    if let Some(bar) = state.bar.take() {
        bar.finish_and_clear();
    }
}

fn plain_line(event: &ProgressEvent, state: &mut RenderState) -> Option<String> {
    match event {
        ProgressEvent::DiscoveryStarted => {
            Some("[1/4] Discovering provider projects and candidate versions...".to_string())
        }
        ProgressEvent::DiscoveringProject {
            provider,
            locator,
            pending_projects,
            artifacts_found,
        } => Some(format!(
            "  {provider}: checking {locator} ({pending_projects} pending, {artifacts_found} JARs found)"
        )),
        ProgressEvent::DiscoveryFinished {
            projects,
            artifacts,
        } => Some(format!(
            "[1/4] Found {artifacts} candidate JARs in {projects} projects."
        )),
        ProgressEvent::CandidateDownloadStarted { total } => Some(format!(
            "[2/4] Downloading, verifying, and parsing {total} candidate JARs..."
        )),
        ProgressEvent::CandidateArtifact {
            completed,
            total,
            filename: _,
            state: ArtifactProgressState::Finished,
        } => Some(format!("  [{completed}/{total}] parsed candidate")),
        ProgressEvent::CandidateArtifact {
            completed,
            total,
            filename: _,
            state: ArtifactProgressState::Failed,
        } => Some(format!("  [{completed}/{total}] candidate failed")),
        ProgressEvent::CandidateDownloadFinished { total } => {
            Some(format!("[2/4] Parsed {total} candidate JARs."))
        }
        ProgressEvent::ResolutionStarted {
            packages,
            candidates,
        } => {
            state.reset_resolution();
            Some(format!(
                "[3/4] Resolving {packages} packages across {candidates} JAR candidates..."
            ))
        }
        ProgressEvent::ResolutionWorkStarted { work } => {
            state.resolution_total += 1;
            Some(format!(
                "  [{}/{}] solver discovered: {}",
                state.resolution_completed,
                state.resolution_total,
                work_started_label(work)
            ))
        }
        ProgressEvent::ResolutionWorkFinished { work } => {
            state.resolution_completed += 1;
            Some(format!(
                "  [{}/{}] solver completed: {}",
                state.resolution_completed,
                state.resolution_total,
                work_finished_label(work)
            ))
        }
        ProgressEvent::ResolutionActivity { activity } => {
            state.record_activity(activity);
            matches!(activity, ResolutionActivity::Solution)
                .then(|| format!("  solver found solution {}", state.solutions))
        }
        ProgressEvent::ResolutionFinished { solutions } => {
            state.solutions = *solutions;
            Some(format!(
                "[3/4] Found {solutions} Pareto-maximal solution(s) · {}.",
                state.resolution_counters()
            ))
        }
        ProgressEvent::ApplyStarted { total } => {
            Some(format!("[4/4] Applying {total} selected packages..."))
        }
        ProgressEvent::ApplyArtifact {
            completed,
            total,
            filename: _,
            state: ArtifactProgressState::Finished,
        } => Some(format!("  [{completed}/{total}] installed package")),
        ProgressEvent::ApplyArtifact {
            completed,
            total,
            filename: _,
            state: ArtifactProgressState::AlreadyPresent,
        } => Some(format!("  [{completed}/{total}] package already present")),
        ProgressEvent::ApplyArtifact {
            completed,
            total,
            filename: _,
            state: ArtifactProgressState::Failed,
        } => Some(format!("  [{completed}/{total}] package failed")),
        ProgressEvent::ApplyFinished { total } => {
            Some(format!("[4/4] Applied/verified {total} selected packages."))
        }
        ProgressEvent::CandidateArtifact {
            state: ArtifactProgressState::Started | ArtifactProgressState::AlreadyPresent,
            ..
        }
        | ProgressEvent::ApplyArtifact {
            state: ArtifactProgressState::Started,
            ..
        } => None,
    }
}

fn audit_plain_line(event: &AuditProgressEvent) -> Option<String> {
    match *event {
        AuditProgressEvent::StageStarted { stage, .. } => Some(format!(
            "[{}/6] {}...",
            audit_stage_number(stage),
            audit_stage_present(stage)
        )),
        AuditProgressEvent::Advanced {
            stage,
            completed,
            total: Some(total),
        } if completed < total && should_report_audit_count(completed, total) => Some(format!(
            "  [{completed}/{total}] {}",
            audit_stage_present(stage).to_ascii_lowercase()
        )),
        AuditProgressEvent::StageFinished { stage, completed } => Some(format!(
            "[{}/6] {}.",
            audit_stage_number(stage),
            audit_stage_finished(stage, completed)
        )),
        AuditProgressEvent::Advanced { .. } => None,
    }
}

fn should_report_audit_count(completed: usize, total: usize) -> bool {
    completed == 1 || completed.is_multiple_of((total / 10).max(1))
}

fn audit_stage_number(stage: AuditProgressStage) -> usize {
    match stage {
        AuditProgressStage::PrepareInputs => 1,
        AuditProgressStage::ScanArtifacts => 2,
        AuditProgressStage::Readiness => 3,
        AuditProgressStage::AnalyzeMixins => 4,
        AuditProgressStage::AnalyzeTransformers => 5,
        AuditProgressStage::DetectConflicts => 6,
    }
}

fn audit_stage_present(stage: AuditProgressStage) -> &'static str {
    match stage {
        AuditProgressStage::PrepareInputs => "Preparing the active runtime classpath",
        AuditProgressStage::Readiness => "Checking audit prerequisites",
        AuditProgressStage::ScanArtifacts => "Scanning bytecode artifacts",
        AuditProgressStage::AnalyzeMixins => "Analyzing Mixins",
        AuditProgressStage::AnalyzeTransformers => "Analyzing Transformers",
        AuditProgressStage::DetectConflicts => "Comparing recovered effects",
    }
}

fn audit_stage_finished(stage: AuditProgressStage, completed: usize) -> String {
    match stage {
        AuditProgressStage::PrepareInputs => "Prepared the active runtime classpath".to_string(),
        AuditProgressStage::Readiness => "Audit prerequisites are ready".to_string(),
        AuditProgressStage::ScanArtifacts => format!("Scanned {completed} bytecode artifacts"),
        AuditProgressStage::AnalyzeMixins => format!("Analyzed {completed} Mixins"),
        AuditProgressStage::AnalyzeTransformers => {
            format!("Analyzed {completed} Transformers")
        }
        AuditProgressStage::DetectConflicts => {
            format!("Detected {completed} compatibility-risk candidates")
        }
    }
}

fn work_started_label(work: &ResolutionWork) -> String {
    match work {
        ResolutionWork::EnumerationRun { run } => format!("search run {run}"),
        ResolutionWork::MaximalityProbe { package } => {
            format!("checking whether {package} can be upgraded")
        }
    }
}

fn work_finished_label(work: &ResolutionWork) -> String {
    match work {
        ResolutionWork::EnumerationRun { run } => format!("search run {run}"),
        ResolutionWork::MaximalityProbe { package } => {
            format!("checked maximality of {package}")
        }
    }
}

fn activity_label(activity: &ResolutionActivity) -> String {
    match activity {
        ResolutionActivity::Decision { package } => format!("deciding {package}"),
        ResolutionActivity::Propagation { package } => format!("propagating {package}"),
        ResolutionActivity::Backtrack {
            from_level,
            to_level,
        } => format!("backtracking {from_level} → {to_level}"),
        ResolutionActivity::Conflict => "resolving a conflict".to_string(),
        ResolutionActivity::Solution => "retained a Pareto-maximal solution".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_progress_reports_candidate_counts_and_failures() {
        let mut state = RenderState::default();
        assert_eq!(
            plain_line(
                &ProgressEvent::CandidateDownloadStarted { total: 42 },
                &mut state
            )
            .as_deref(),
            Some("[2/4] Downloading, verifying, and parsing 42 candidate JARs...")
        );
        assert_eq!(
            plain_line(
                &ProgressEvent::CandidateArtifact {
                    completed: 7,
                    total: 42,
                    filename: "voxy.jar".to_string(),
                    state: ArtifactProgressState::Finished,
                },
                &mut state,
            )
            .as_deref(),
            Some("  [7/42] parsed candidate")
        );
        assert_eq!(
            plain_line(
                &ProgressEvent::CandidateArtifact {
                    completed: 8,
                    total: 42,
                    filename: "dependency.jar".to_string(),
                    state: ArtifactProgressState::Failed,
                },
                &mut state,
            )
            .as_deref(),
            Some("  [8/42] candidate failed")
        );
        assert!(
            !plain_line(
                &ProgressEvent::CandidateArtifact {
                    completed: 9,
                    total: 42,
                    filename: "must-not-be-rendered.jar".to_string(),
                    state: ArtifactProgressState::Finished,
                },
                &mut state,
            )
            .unwrap()
            .contains(".jar")
        );
    }

    #[test]
    fn solver_progress_grows_its_total_as_work_is_discovered() {
        let mut state = RenderState::default();
        plain_line(
            &ProgressEvent::ResolutionStarted {
                packages: 36,
                candidates: 184,
            },
            &mut state,
        );
        let first = plain_line(
            &ProgressEvent::ResolutionWorkStarted {
                work: ResolutionWork::EnumerationRun { run: 1 },
            },
            &mut state,
        );
        plain_line(
            &ProgressEvent::ResolutionWorkFinished {
                work: ResolutionWork::EnumerationRun { run: 1 },
            },
            &mut state,
        );
        let second = plain_line(
            &ProgressEvent::ResolutionWorkStarted {
                work: ResolutionWork::MaximalityProbe {
                    package: "sodium".to_string(),
                },
            },
            &mut state,
        );

        assert_eq!(
            first.as_deref(),
            Some("  [0/1] solver discovered: search run 1")
        );
        assert_eq!(
            second.as_deref(),
            Some("  [1/2] solver discovered: checking whether sodium can be upgraded")
        );
        assert_eq!(state.resolution_completed, 1);
        assert_eq!(state.resolution_total, 2);
    }

    #[test]
    fn plain_audit_progress_reports_truthful_stage_counts_without_artifact_names() {
        let start = audit_plain_line(&AuditProgressEvent::StageStarted {
            stage: AuditProgressStage::ScanArtifacts,
            total: Some(100),
        });
        let progress = audit_plain_line(&AuditProgressEvent::Advanced {
            stage: AuditProgressStage::ScanArtifacts,
            completed: 10,
            total: Some(100),
        });
        let noisy = audit_plain_line(&AuditProgressEvent::Advanced {
            stage: AuditProgressStage::ScanArtifacts,
            completed: 11,
            total: Some(100),
        });

        assert_eq!(
            start.as_deref(),
            Some("[2/6] Scanning bytecode artifacts...")
        );
        assert_eq!(
            progress.as_deref(),
            Some("  [10/100] scanning bytecode artifacts")
        );
        assert!(noisy.is_none());
    }
}
