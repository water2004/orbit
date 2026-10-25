//! Bounded presentation model for the CLI audit response.
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct AuditFinding {
    pub packages: String,
    pub rule: String,
    pub reason: String,
    pub risk: u8,
    pub severity: String,
    pub confidence: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct AuditNotice {
    pub artifact: Option<String>,
    pub scope: String,
    pub kind: String,
    pub detail: String,
    pub count: usize,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct AuditSummary {
    pub readiness: String,
    pub readiness_message: String,
    pub loader: Option<String>,
    pub runtime_namespace: Option<String>,
    pub capabilities: Vec<String>,
    pub artifacts: usize,
    pub warnings: Vec<AuditNotice>,
    pub coverage_gaps: Vec<AuditNotice>,
    pub findings: Vec<AuditFinding>,
}
