use std::collections::{BTreeSet, HashMap};

use comfy_table::Cell;

use super::output_table;

pub fn audit_report(report: &orbit_core::AuditReport, limit: usize) -> String {
    let mut sections = vec![environment_table(report), summary_table(report)];

    if report.risks.is_empty() && report.unary_risks.is_empty() {
        sections.push("未发现达到当前阈值的字节码兼容风险。".to_string());
        sections.push(
            "Use --format json or --report <path> for the complete structured report.".to_string(),
        );
    } else {
        sections.push(format!(
            "Structural compatibility risks: {}",
            report.risks.len() + report.unary_risks.len()
        ));
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
    let mut table = output_table(["Minecraft", "Loader", "Runtime symbols"]);
    let namespace = report
        .namespace
        .runtime_namespace
        .map(namespace_label)
        .unwrap_or("unknown");
    let mapping = report
        .namespace
        .mapping_sources
        .first()
        .map(|source| source.resource_path.as_str())
        .unwrap_or("identity");
    table.add_row([
        Cell::new(&report.environment.minecraft_version),
        Cell::new(format!(
            "{} {}",
            report.environment.detected_loader, report.environment.loader_version
        )),
        Cell::new(format!("{namespace}\n{mapping}\nalignment complete")),
    ]);
    format!("Bytecode audit\n{table}")
}

fn summary_table(report: &orbit_core::AuditReport) -> String {
    let mut table = output_table([
        "Structural risks",
        "Behavioral interactions",
        "Coverage gaps",
        "Warnings",
    ]);
    table.add_row([
        Cell::new(report.risks.len() + report.unary_risks.len()),
        Cell::new(report.interactions.len()),
        Cell::new(
            report
                .coverage_gaps
                .iter()
                .map(|gap| gap.count)
                .sum::<usize>(),
        ),
        Cell::new(report.warnings.len()),
    ]);
    format!("Summary\n{table}")
}

enum DisplayRisk<'a> {
    Unary(&'a orbit_core::AuditUnaryRisk),
    Pair(&'a orbit_core::AuditRisk),
}

impl DisplayRisk<'_> {
    fn risk_index(&self) -> u8 {
        match self {
            Self::Unary(risk) => risk.risk_index,
            Self::Pair(risk) => risk.risk_index,
        }
    }
}

fn risks_table(report: &orbit_core::AuditReport, limit: usize) -> String {
    let labels = report
        .artifacts
        .iter()
        .map(|artifact| (artifact.id.as_str(), artifact.display_name.as_str()))
        .collect::<HashMap<_, _>>();
    let total = report.risks.len() + report.unary_risks.len();
    let mut ranked = report
        .unary_risks
        .iter()
        .map(DisplayRisk::Unary)
        .chain(report.risks.iter().map(DisplayRisk::Pair))
        .enumerate()
        .collect::<Vec<_>>();
    ranked.sort_by(|(left_order, left), (right_order, right)| {
        right
            .risk_index()
            .cmp(&left.risk_index())
            .then_with(|| left_order.cmp(right_order))
    });
    let shown = total.min(limit);
    let mut table = output_table(["Risk", "Details"]);

    for (index, (_, risk)) in ranked.into_iter().take(limit).enumerate() {
        let (risk_index, details) = match risk {
            DisplayRisk::Unary(risk) => {
                let artifact = labels
                    .get(risk.artifact_id.as_str())
                    .copied()
                    .unwrap_or(&risk.artifact_id);
                (
                    risk.risk_index,
                    format!(
                        "Package: {artifact}\nEnvironment: {}\nTarget: {}\nReason: {}\nRule: {}\nImpact: {} · Confidence: {} · Activation: {} · Precision: {}",
                        risk.environment_target,
                        format_target(&risk.target),
                        risk.reason,
                        risk.rule,
                        severity_label(risk.severity).to_ascii_lowercase(),
                        confidence_label(risk.confidence),
                        activation_label(risk.activation),
                        precision_label(risk.precision),
                    ),
                )
            }
            DisplayRisk::Pair(risk) => {
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
                    "Packages: {left} ↔ {right}\nTarget: {}\nReason: {}\nRule: {}\nImpact: {} · Confidence: {} · Activation: {} · Precision: {}",
                    format_target(&risk.target),
                    risk.reason,
                    risk.rule,
                    severity_label(risk.severity).to_ascii_lowercase(),
                    confidence_label(risk.confidence),
                    activation_label(risk.activation),
                    precision_label(risk.precision),
                );
                if !sources.is_empty() {
                    details.push_str("\nSource: ");
                    details.push_str(&sources.into_iter().collect::<Vec<_>>().join(" × "));
                }
                (risk.risk_index, details)
            }
        };
        table.add_row([
            Cell::new(format!("#{}\nRISK {risk_index}", index + 1)),
            Cell::new(details),
        ]);
    }

    format!("Risks (showing {shown} of {total})\n{table}")
}

fn precision_label(precision: orbit_core::AuditPrecision) -> &'static str {
    match precision {
        orbit_core::AuditPrecision::Instruction => "instruction",
        orbit_core::AuditPrecision::Pattern => "pattern",
        orbit_core::AuditPrecision::Method => "method",
        orbit_core::AuditPrecision::Class => "class",
        orbit_core::AuditPrecision::Unknown => "unknown",
    }
}

fn format_target(target: &orbit_core::audit_model::Target) -> String {
    target.member.as_ref().map_or_else(
        || target.class.clone(),
        |member| format!("{}::{}{}", target.class, member.name, member.descriptor),
    )
}

fn namespace_label(namespace: orbit_core::AuditSymbolNamespace) -> &'static str {
    match namespace {
        orbit_core::AuditSymbolNamespace::Runtime => "runtime",
        orbit_core::AuditSymbolNamespace::Official => "official",
        orbit_core::AuditSymbolNamespace::Intermediary => "intermediary",
        orbit_core::AuditSymbolNamespace::Srg => "srg",
        orbit_core::AuditSymbolNamespace::Named => "named",
        orbit_core::AuditSymbolNamespace::Identity => "identity",
        orbit_core::AuditSymbolNamespace::Unknown => "unknown",
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
