use std::io::IsTerminal;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use indicatif::{ProgressBar, ProgressDrawTarget, ProgressStyle};
use orbit_core::{
    ArtifactProgressState, AuditProgressEvent, AuditProgressReporter, AuditProgressStage,
    ProgressBarMode, ProgressEvent, ProgressReporter, ResolutionCurrent,
};

pub fn reporter(quiet: bool, configured_style: ProgressBarMode) -> Option<ProgressReporter> {
    if quiet || configured_style == ProgressBarMode::Off {
        return None;
    }

    let modern = configured_style == ProgressBarMode::Modern && std::io::stderr().is_terminal();
    let renderer = Arc::new(ProgressRenderer {
        modern,
        state: Mutex::new(RenderState::default()),
    });
    Some(Arc::new(move |event| renderer.render(event)))
}

pub fn audit_reporter(
    quiet: bool,
    configured_style: ProgressBarMode,
) -> Option<AuditProgressReporter> {
    if quiet || configured_style == ProgressBarMode::Off {
        return None;
    }

    let modern = configured_style == ProgressBarMode::Modern && std::io::stderr().is_terminal();
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
    resolution_total: u64,
    resolution_completed: u64,
    decisions: u64,
    propagations: u64,
    backtracks: u64,
    conflicts: u64,
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
            ProgressEvent::RepositoryIndexStarted {
                minecraft,
                loader,
                total,
            } => start_bar(
                &mut state,
                total,
                tr!(
                    "[1/4] Checking the local version repository for Minecraft %{minecraft} / %{loader}",
                    minecraft = minecraft,
                    loader = loader
                ),
            ),
            ProgressEvent::RepositoryProjectChecked {
                completed,
                total,
                provider,
                project_id,
                refreshed,
                artifacts,
            } => {
                if let Some(bar) = &state.bar {
                    bar.set_length(total as u64);
                    bar.set_position(completed as u64);
                    bar.set_message(tr!(
                        "[1/4] %{provider}:%{project} · %{state} · %{artifacts} candidate JAR(s)",
                        provider = provider,
                        project = project_id,
                        state = if refreshed {
                            tr!("refreshed")
                        } else {
                            tr!("reused")
                        },
                        artifacts = artifacts
                    ));
                }
            }
            ProgressEvent::RepositoryIndexFinished {
                completed: _,
                total,
                refreshed,
                reused,
                artifacts,
            } => finish(
                &mut state,
                tr!(
                    "[1/4] Indexed %{total} projects (%{refreshed} refreshed, %{reused} reused) and %{artifacts} candidate JARs",
                    total = total,
                    refreshed = refreshed,
                    reused = reused,
                    artifacts = artifacts
                ),
            ),
            ProgressEvent::CandidateDownloadStarted { total } => start_bar(
                &mut state,
                total,
                tr!(
                    "[2/4] Downloading, verifying, and parsing %{total} candidate JARs",
                    total = total
                ),
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
                        ArtifactProgressState::Started => tr!("processing"),
                        ArtifactProgressState::Finished => tr!("parsed"),
                        ArtifactProgressState::AlreadyPresent => tr!("cached"),
                        ArtifactProgressState::Failed => tr!("failed"),
                    };
                    bar.set_message(tr!(
                        "[2/4] %{action} candidate %{completed}/%{total}",
                        action = action,
                        completed = completed,
                        total = total
                    ));
                }
            }
            ProgressEvent::CandidateDownloadFinished { total } => finish(
                &mut state,
                tr!(
                    "[2/4] Downloaded/verified and parsed %{total} candidate JARs",
                    total = total
                ),
            ),
            ProgressEvent::ResolutionStarted {
                packages,
                candidates,
            } => {
                state.reset_resolution();
                start_bar(
                    &mut state,
                    0,
                    tr!(
                        "[3/4] Resolving %{packages} packages across %{candidates} JAR candidates",
                        packages = packages,
                        candidates = candidates
                    ),
                );
            }
            ProgressEvent::ResolutionAdvanced {
                work_discovered,
                work_completed,
                decisions,
                propagations,
                backtracks,
                conflicts,
                solutions,
                current,
            } => {
                state.resolution_total = work_discovered;
                state.resolution_completed = work_completed;
                state.decisions = decisions;
                state.propagations = propagations;
                state.backtracks = backtracks;
                state.conflicts = conflicts;
                state.solutions = solutions;
                update_resolution_bar(
                    &state,
                    format!(
                        "[3/4] {} · {}",
                        current
                            .as_ref()
                            .map(resolution_current_label)
                            .unwrap_or_else(|| tr!("resolving dependencies").into_owned()),
                        state.resolution_counters()
                    ),
                );
            }
            ProgressEvent::ResolutionFinished { solutions } => {
                state.solutions = solutions;
                let counters = state.resolution_counters();
                finish(
                    &mut state,
                    tr!(
                        "[3/4] Found %{solutions} non-dominated solution(s) · %{counters}",
                        solutions = solutions,
                        counters = counters
                    ),
                );
            }
            ProgressEvent::ApplyStarted { total } => start_bar(
                &mut state,
                total,
                tr!("[4/4] Applying %{total} selected packages", total = total),
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
                        ArtifactProgressState::Started => tr!("applying"),
                        ArtifactProgressState::Finished => tr!("installed"),
                        ArtifactProgressState::AlreadyPresent => tr!("already present"),
                        ArtifactProgressState::Failed => tr!("failed"),
                    };
                    bar.set_message(tr!(
                        "[4/4] %{action} package %{completed}/%{total}",
                        action = action,
                        completed = completed,
                        total = total
                    ));
                }
            }
            ProgressEvent::ApplyFinished { total } => finish(
                &mut state,
                tr!(
                    "[4/4] Applied/verified %{total} selected packages",
                    total = total
                ),
            ),
            ProgressEvent::ExportStarted {
                packages,
                total_bytes,
            } => start_bar(
                &mut state,
                usize::try_from(total_bytes).unwrap_or(usize::MAX),
                tr!("Exporting %{packages} package(s)", packages = packages),
            ),
            ProgressEvent::ExportAdvanced {
                completed,
                total,
                completed_packages,
                packages,
            } => {
                if let Some(bar) = &state.bar {
                    bar.set_length(total);
                    bar.set_position(completed);
                    bar.set_message(tr!(
                        "Exporting packages %{completed}/%{total}",
                        completed = completed_packages,
                        total = packages
                    ));
                }
            }
            ProgressEvent::ExportFinished { packages, .. } => finish(
                &mut state,
                tr!("Exported %{packages} package(s)", packages = packages),
            ),
            ProgressEvent::ImportStarted { files, total_bytes } => start_bar(
                &mut state,
                usize::try_from(total_bytes).unwrap_or(usize::MAX),
                tr!("Verifying and importing %{files} file(s)", files = files),
            ),
            ProgressEvent::ImportAdvanced {
                completed_bytes,
                total_bytes,
                completed_files,
                files,
            } => {
                if let Some(bar) = &state.bar {
                    bar.set_length(total_bytes);
                    bar.set_position(completed_bytes);
                    bar.set_message(tr!(
                        "Importing files %{completed}/%{total}",
                        completed = completed_files,
                        total = files
                    ));
                }
            }
            ProgressEvent::ImportFinished { files, .. } => {
                finish(&mut state, tr!("Imported %{files} file(s)", files = files))
            }
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

    fn resolution_counters(&self) -> String {
        tr!(
            "work %{completed}/%{total}, %{decisions} decisions, %{propagations} propagations, %{backtracks} backtracks, %{conflicts} conflicts, %{solutions} solutions",
            completed = self.resolution_completed,
            total = self.resolution_total,
            decisions = self.decisions,
            propagations = self.propagations,
            backtracks = self.backtracks,
            conflicts = self.conflicts,
            solutions = self.solutions
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

fn update_resolution_bar(state: &RenderState, message: String) {
    if let Some(bar) = &state.bar {
        bar.set_length(state.resolution_total);
        bar.set_position(state.resolution_completed);
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
        ProgressEvent::RepositoryIndexStarted {
            minecraft,
            loader,
            total,
        } => Some(tr!(
            "[1/4] Checking %{total} project(s) in the local version repository for Minecraft %{minecraft} / %{loader}...",
            total = total,
            minecraft = minecraft,
            loader = loader
        )),
        ProgressEvent::RepositoryProjectChecked {
            completed,
            total,
            provider,
            project_id,
            refreshed,
            artifacts,
        } => Some(tr!(
            "  [%{completed}/%{total}] %{provider}:%{project} · %{state} · %{artifacts} candidate JAR(s)",
            completed = completed,
            total = total,
            provider = provider,
            project = project_id,
            state = if *refreshed {
                tr!("refreshed")
            } else {
                tr!("reused")
            },
            artifacts = artifacts
        )),
        ProgressEvent::RepositoryIndexFinished {
            total,
            refreshed,
            reused,
            artifacts,
            ..
        } => Some(tr!(
            "[1/4] Indexed %{total} projects (%{refreshed} refreshed, %{reused} reused) and %{artifacts} candidate JARs.",
            total = total,
            refreshed = refreshed,
            reused = reused,
            artifacts = artifacts
        )),
        ProgressEvent::CandidateDownloadStarted { total } => Some(tr!(
            "[2/4] Downloading, verifying, and parsing %{total} candidate JARs...",
            total = total
        )),
        ProgressEvent::CandidateArtifact {
            completed,
            total,
            filename: _,
            state: ArtifactProgressState::Finished,
        } => Some(tr!(
            "  [%{completed}/%{total}] parsed candidate",
            completed = completed,
            total = total
        )),
        ProgressEvent::CandidateArtifact {
            completed,
            total,
            filename: _,
            state: ArtifactProgressState::Failed,
        } => Some(tr!(
            "  [%{completed}/%{total}] candidate failed",
            completed = completed,
            total = total
        )),
        ProgressEvent::CandidateDownloadFinished { total } => {
            Some(tr!("[2/4] Parsed %{total} candidate JARs.", total = total))
        }
        ProgressEvent::ResolutionStarted {
            packages,
            candidates,
        } => {
            state.reset_resolution();
            Some(tr!(
                "[3/4] Resolving %{packages} packages across %{candidates} JAR candidates...",
                packages = packages,
                candidates = candidates
            ))
        }
        ProgressEvent::ResolutionAdvanced {
            work_discovered,
            work_completed,
            decisions,
            propagations,
            backtracks,
            conflicts,
            solutions,
            current,
        } => {
            state.resolution_total = *work_discovered;
            state.resolution_completed = *work_completed;
            state.decisions = *decisions;
            state.propagations = *propagations;
            state.backtracks = *backtracks;
            state.conflicts = *conflicts;
            state.solutions = *solutions;
            let current = current
                .as_ref()
                .map(resolution_current_label)
                .unwrap_or_else(|| tr!("resolving dependencies").into_owned());
            Some(tr!(
                "  [%{completed}/%{total}] %{current} · %{counters}",
                completed = work_completed,
                total = work_discovered,
                current = current,
                counters = state.resolution_counters()
            ))
        }
        ProgressEvent::ResolutionFinished { solutions } => {
            state.solutions = *solutions;
            Some(tr!(
                "[3/4] Found %{solutions} non-dominated solution(s) · %{counters}.",
                solutions = solutions,
                counters = state.resolution_counters()
            ))
        }
        ProgressEvent::ApplyStarted { total } => Some(tr!(
            "[4/4] Applying %{total} selected packages...",
            total = total
        )),
        ProgressEvent::ApplyArtifact {
            completed,
            total,
            filename: _,
            state: ArtifactProgressState::Finished,
        } => Some(tr!(
            "  [%{completed}/%{total}] installed package",
            completed = completed,
            total = total
        )),
        ProgressEvent::ApplyArtifact {
            completed,
            total,
            filename: _,
            state: ArtifactProgressState::AlreadyPresent,
        } => Some(tr!(
            "  [%{completed}/%{total}] package already present",
            completed = completed,
            total = total
        )),
        ProgressEvent::ApplyArtifact {
            completed,
            total,
            filename: _,
            state: ArtifactProgressState::Failed,
        } => Some(tr!(
            "  [%{completed}/%{total}] package failed",
            completed = completed,
            total = total
        )),
        ProgressEvent::ApplyFinished { total } => Some(tr!(
            "[4/4] Applied/verified %{total} selected packages.",
            total = total
        )),
        ProgressEvent::ExportStarted { packages, .. } => Some(tr!(
            "Exporting %{packages} package(s)...",
            packages = packages
        )),
        ProgressEvent::ExportAdvanced { .. } => None,
        ProgressEvent::ExportFinished { packages, .. } => {
            Some(tr!("Exported %{packages} package(s).", packages = packages))
        }
        ProgressEvent::ImportStarted { files, .. } => Some(tr!(
            "Verifying and importing %{files} file(s)...",
            files = files
        )),
        ProgressEvent::ImportAdvanced { .. } => None,
        ProgressEvent::ImportFinished { files, .. } => {
            Some(tr!("Imported %{files} file(s).", files = files))
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

fn audit_stage_present(stage: AuditProgressStage) -> std::borrow::Cow<'static, str> {
    match stage {
        AuditProgressStage::PrepareInputs => tr!("Preparing the active runtime classpath"),
        AuditProgressStage::Readiness => tr!("Checking audit prerequisites"),
        AuditProgressStage::ScanArtifacts => tr!("Scanning bytecode artifacts"),
        AuditProgressStage::AnalyzeMixins => tr!("Analyzing Mixins"),
        AuditProgressStage::AnalyzeTransformers => tr!("Analyzing Transformers"),
        AuditProgressStage::DetectConflicts => tr!("Comparing recovered effects"),
    }
}

fn audit_stage_finished(stage: AuditProgressStage, completed: usize) -> String {
    match stage {
        AuditProgressStage::PrepareInputs => {
            tr!("Prepared the active runtime classpath").into_owned()
        }
        AuditProgressStage::Readiness => tr!("Audit prerequisites are ready").into_owned(),
        AuditProgressStage::ScanArtifacts => tr!(
            "Scanned %{completed} bytecode artifacts",
            completed = completed
        ),
        AuditProgressStage::AnalyzeMixins => {
            tr!("Analyzed %{completed} Mixins", completed = completed)
        }
        AuditProgressStage::AnalyzeTransformers => {
            tr!("Analyzed %{completed} Transformers", completed = completed)
        }
        AuditProgressStage::DetectConflicts => {
            tr!(
                "Detected %{completed} compatibility-risk candidates",
                completed = completed
            )
        }
    }
}

fn resolution_current_label(current: &ResolutionCurrent) -> String {
    match current {
        ResolutionCurrent::Enumeration { run } => {
            tr!("Searching solution space (run %{run})", run = run)
        }
        ResolutionCurrent::VersionMaximization { package } => {
            tr!(
                "Checking whether %{package} can be upgraded",
                package = package
            )
        }
        ResolutionCurrent::PreferencePreservation { package } => {
            tr!(
                "Checking whether %{package} can be preserved",
                package = package
            )
        }
        ResolutionCurrent::Decision { package } => {
            tr!("Deciding %{package}", package = package)
        }
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
            &ProgressEvent::ResolutionAdvanced {
                work_discovered: 1,
                work_completed: 0,
                decisions: 3,
                propagations: 8,
                backtracks: 0,
                conflicts: 0,
                solutions: 0,
                current: Some(ResolutionCurrent::Enumeration { run: 1 }),
            },
            &mut state,
        );
        let second = plain_line(
            &ProgressEvent::ResolutionAdvanced {
                work_discovered: 2,
                work_completed: 1,
                decisions: 7,
                propagations: 20,
                backtracks: 1,
                conflicts: 1,
                solutions: 0,
                current: Some(ResolutionCurrent::VersionMaximization {
                    package: "sodium".to_string(),
                }),
            },
            &mut state,
        );

        assert!(
            first
                .unwrap()
                .contains("[0/1] Searching solution space (run 1)")
        );
        assert!(
            second
                .unwrap()
                .contains("[1/2] Checking whether sodium can be upgraded")
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
