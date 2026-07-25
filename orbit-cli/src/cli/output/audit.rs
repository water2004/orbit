use std::collections::{BTreeMap, BTreeSet, HashMap};

use comfy_table::Cell;

use super::output_table;

pub fn audit_report(report: &orbit_core::AuditReport, limit: usize) -> String {
    let mut sections = vec![environment_table(report), coverage_table(&report.coverage)];

    if !report.coverage.unsupported_mechanisms.is_empty()
        || !report.coverage.budget_exhaustions.is_empty()
    {
        sections.push(coverage_gaps_table(&report.coverage));
    }
    if !report.warnings.is_empty() {
        sections.push(warnings_table(report));
    }

    if report.risks.is_empty() {
        sections.push("未发现达到当前阈值的字节码兼容风险。".to_string());
        sections.push(
            "Use --format json or --report <path> for the complete structured report.".to_string(),
        );
    } else {
        sections.push(risk_distribution_table(report));
        sections.push(risks_table(report, limit));
        sections.push(
            "Use --format json or --report <path> for all evidence, selectors, warnings, and offsets."
                .to_string(),
        );
    }

    let mut output = sections.join("\n\n");
    output.push('\n');
    output
}

fn environment_table(report: &orbit_core::AuditReport) -> String {
    let mut table = output_table(["Minecraft", "Loader", "Readiness"]);
    table.add_row([
        Cell::new(&report.environment.minecraft_version),
        Cell::new(format!(
            "{} {}",
            report.environment.detected_loader, report.environment.loader_version
        )),
        Cell::new(format!(
            "{}\n{}",
            readiness_label(report.readiness.status),
            report.readiness.message
        )),
    ]);
    format!("Bytecode audit\n{table}")
}

fn coverage_table(coverage: &orbit_core::AuditCoverage) -> String {
    let mut table = output_table(["Area", "Count", "Coverage notes"]);
    table.add_row([
        Cell::new("JARs"),
        Cell::new(coverage.jars_scanned),
        Cell::new(format!("{} failed", coverage.jars_failed)),
    ]);
    table.add_row([
        Cell::new("Classes"),
        Cell::new(format!(
            "{} / {} parsed",
            coverage.classes_parsed, coverage.classes_discovered
        )),
        Cell::new(format!("{} failed", coverage.classes_failed)),
    ]);
    table.add_row([
        Cell::new("Methods"),
        Cell::new(format!("{} parsed", coverage.methods_parsed)),
        Cell::new(format!("{} degraded", coverage.methods_degraded)),
    ]);
    table.add_row([
        Cell::new("Mixins"),
        Cell::new(coverage.mixins_discovered),
        Cell::new(format!(
            "{} instruction/pattern, {} method, {} class/unknown effects",
            coverage.effects_instruction_precision,
            coverage.effects_method_precision,
            coverage.effects_class_precision
        )),
    ]);
    table.add_row([
        Cell::new("Transformers"),
        Cell::new(coverage.transformers_discovered),
        Cell::new(format!(
            "{} targets; {} exact, {} partial, {} unknown effects",
            coverage.transformer_targets_recovered,
            coverage.transformer_effects_recovered,
            coverage.transformer_effects_partial,
            coverage.transformer_effects_unknown
        )),
    ]);
    format!("Coverage\n{table}")
}

fn coverage_gaps_table(coverage: &orbit_core::AuditCoverage) -> String {
    let mut table = output_table(["Category", "Details"]);
    for mechanism in &coverage.unsupported_mechanisms {
        table.add_row([Cell::new("Unsupported mechanism"), Cell::new(mechanism)]);
    }
    for exhaustion in &coverage.budget_exhaustions {
        table.add_row([
            Cell::new("Analysis budget exhausted"),
            Cell::new(exhaustion),
        ]);
    }
    format!("Coverage gaps\n{table}")
}

fn warnings_table(report: &orbit_core::AuditReport) -> String {
    let mut counts = BTreeMap::new();
    for warning in &report.warnings {
        *counts.entry(warning.kind).or_insert(0_usize) += 1;
    }

    let mut table = output_table(["Warning", "Count"]);
    for (kind, count) in counts {
        table.add_row([Cell::new(warning_label(kind)), Cell::new(count)]);
    }
    format!("Warnings ({})\n{table}", report.warnings.len())
}

fn risk_distribution_table(report: &orbit_core::AuditReport) -> String {
    let mut table = output_table(["Critical", "High", "Medium", "Low"]);
    table.add_row(
        [
            orbit_core::AuditSeverity::Critical,
            orbit_core::AuditSeverity::High,
            orbit_core::AuditSeverity::Medium,
            orbit_core::AuditSeverity::Low,
        ]
        .map(|severity| {
            Cell::new(
                report
                    .risks
                    .iter()
                    .filter(|risk| risk.severity == severity)
                    .count(),
            )
        }),
    );
    format!("Risk distribution\n{table}")
}

fn risks_table(report: &orbit_core::AuditReport, limit: usize) -> String {
    let labels = report
        .artifacts
        .iter()
        .map(|artifact| (artifact.id.as_str(), artifact.display_name.as_str()))
        .collect::<HashMap<_, _>>();
    let shown = report.risks.len().min(limit);
    let mut table = output_table(["Risk", "Details"]);

    for (index, risk) in report.risks.iter().take(limit).enumerate() {
        let left = labels
            .get(risk.left_artifact.as_str())
            .copied()
            .unwrap_or(&risk.left_artifact);
        let right = labels
            .get(risk.right_artifact.as_str())
            .copied()
            .unwrap_or(&risk.right_artifact);
        let sources = risk
            .evidence
            .iter()
            .filter_map(|evidence| {
                evidence.mechanism.map(|mechanism| {
                    evidence.injector_kind.as_ref().map_or_else(
                        || mechanism_label(mechanism).to_string(),
                        |injector| format!("{} {injector}", mechanism_label(mechanism)),
                    )
                })
            })
            .collect::<BTreeSet<_>>();
        let mut details = format!(
            "Packages: {left} ↔ {right}\nTarget: {}\nReason: {}\nRule: {}\nConfidence: {} · Activation: {}",
            format_target(&risk.target),
            risk.reason,
            risk.rule,
            confidence_label(risk.confidence),
            activation_label(risk.activation)
        );
        if !sources.is_empty() {
            details.push_str("\nSource: ");
            details.push_str(&sources.into_iter().collect::<Vec<_>>().join(" × "));
        }

        table.add_row([
            Cell::new(format!(
                "#{}\n{}\nscore {}",
                index + 1,
                severity_label(risk.severity),
                risk.risk_index
            )),
            Cell::new(details),
        ]);
    }

    format!("Risks (showing {shown} of {})\n{table}", report.risks.len())
}

fn format_target(target: &orbit_core::audit_model::Target) -> String {
    target.member.as_ref().map_or_else(
        || target.class.clone(),
        |member| format!("{}::{}{}", target.class, member.name, member.descriptor),
    )
}

fn readiness_label(status: orbit_core::AuditReadinessStatus) -> &'static str {
    match status {
        orbit_core::AuditReadinessStatus::Ready => "Ready",
        orbit_core::AuditReadinessStatus::Unsupported => "Unsupported",
        orbit_core::AuditReadinessStatus::Incomplete => "Incomplete",
        orbit_core::AuditReadinessStatus::Ambiguous => "Ambiguous",
    }
}

fn severity_label(severity: orbit_core::AuditSeverity) -> &'static str {
    match severity {
        orbit_core::AuditSeverity::Low => "LOW",
        orbit_core::AuditSeverity::Medium => "MEDIUM",
        orbit_core::AuditSeverity::High => "HIGH",
        orbit_core::AuditSeverity::Critical => "CRITICAL",
    }
}

fn confidence_label(confidence: orbit_core::AuditConfidence) -> &'static str {
    match confidence {
        orbit_core::AuditConfidence::Low => "low",
        orbit_core::AuditConfidence::Medium => "medium",
        orbit_core::AuditConfidence::High => "high",
        orbit_core::AuditConfidence::Exact => "exact",
    }
}

fn activation_label(activation: orbit_core::AuditActivation) -> &'static str {
    match activation {
        orbit_core::AuditActivation::Definite => "definite",
        orbit_core::AuditActivation::Conditional => "conditional",
        orbit_core::AuditActivation::Candidate => "candidate",
        orbit_core::AuditActivation::Unknown => "unknown",
    }
}

fn mechanism_label(mechanism: orbit_core::AuditMechanism) -> &'static str {
    match mechanism {
        orbit_core::AuditMechanism::Mixin => "Mixin",
        orbit_core::AuditMechanism::MixinExtras => "MixinExtras",
        orbit_core::AuditMechanism::ModLauncherTransformer => "ModLauncher transformer",
        orbit_core::AuditMechanism::JavaCoremod => "Java coremod",
        orbit_core::AuditMechanism::BinaryShape => "binary shape",
    }
}

fn warning_label(kind: orbit_core::AuditWarningKind) -> &'static str {
    match kind {
        orbit_core::AuditWarningKind::UnresolvedSoftReference => "Unresolved soft references",
        orbit_core::AuditWarningKind::AmbiguousSoftReference => "Ambiguous soft references",
        orbit_core::AuditWarningKind::KnownUnsupportedInjectionPoint => {
            "Known but unsupported injection points"
        }
        orbit_core::AuditWarningKind::CustomInjectionPoint => "Custom injection points",
        orbit_core::AuditWarningKind::DamagedArtifact => "Damaged artifacts",
        orbit_core::AuditWarningKind::DamagedClass => "Damaged classes",
        orbit_core::AuditWarningKind::TransformerPartial => "Partial transformer analyses",
        orbit_core::AuditWarningKind::UnsupportedMechanism => "Unsupported mechanisms",
        orbit_core::AuditWarningKind::BudgetExhaustion => "Analysis budget exhaustions",
        orbit_core::AuditWarningKind::Other => "Other warnings",
    }
}
