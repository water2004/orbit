use std::collections::HashMap;

/// 包标识符
pub type PackageId = String;

#[derive(Debug, Clone)]
pub struct CandidateVersion {
    pub jar_version: String,
    pub deps: Vec<(String, String, bool)>,
    pub implanted: Vec<ImplantedCandidate>,
}

#[derive(Debug, Clone)]
pub struct ImplantedCandidate {
    pub mod_id: String,
    pub version: String,
    pub deps: Vec<(String, String, bool)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CandidateDiagnosticKind {
    ExcludedByPropagation,
    Backtracked,
    ProviderPreferred,
    Unexplained,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateDiagnostic {
    pub package: String,
    pub selected_version: String,
    pub candidate_version: String,
    pub kind: CandidateDiagnosticKind,
    pub preferred_version: Option<String>,
    pub facts: Vec<String>,
}

impl std::fmt::Display for CandidateDiagnostic {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind {
            CandidateDiagnosticKind::ExcludedByPropagation => write!(
                f,
                "{} stayed at {}; candidate {} was excluded by dependency propagation",
                self.package, self.selected_version, self.candidate_version
            )?,
            CandidateDiagnosticKind::Backtracked => write!(
                f,
                "{} stayed at {}; candidate {} was tried, then backtracked after a conflict",
                self.package, self.selected_version, self.candidate_version
            )?,
            CandidateDiagnosticKind::ProviderPreferred => write!(
                f,
                "{} stayed at {}; candidate {} was allowed, but version selection preferred {}",
                self.package,
                self.selected_version,
                self.candidate_version,
                self.preferred_version.as_deref().unwrap_or("?")
            )?,
            CandidateDiagnosticKind::Unexplained => write!(
                f,
                "{} stayed at {}; candidate {} was not selected, but no excluding derivation was recorded",
                self.package, self.selected_version, self.candidate_version
            )?,
        }
        for fact in &self.facts {
            write!(f, "\n  - {fact}")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct ResolutionReport {
    pub upgrades: HashMap<String, String>,
    pub diagnostics: Vec<CandidateDiagnostic>,
}
