use std::collections::{BTreeSet, HashMap};

use comfy_table::Cell;

use super::output_table;

pub fn audit_report(report: &orbit_core::AuditReport, limit: usize) -> String {
    let mut sections = vec![environment_table(report), summary_table(report)];

    if report.risks.is_empty() && report.unary_risks.is_empty() {
        sections.push(
            tr!("No bytecode compatibility risks reached the current threshold.").into_owned(),
        );
        sections.push(
            tr!("Use --format json or --report <path> for the complete structured report.")
                .into_owned(),
        );
    } else {
        sections.push(tr!(
            "Structural compatibility risks: %{count}",
            count = report.risks.len() + report.unary_risks.len()
        ));
        sections.push(risks_table(report, limit));
        sections.push(
            tr!("Use --format json or --report <path> for all evidence, selectors, warnings, and offsets.").into_owned(),
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
        .unwrap_or_else(|| tr!("unknown"));
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
            report.environment.loader, report.environment.loader_version
        )),
        Cell::new(format!(
            "{namespace}\n{mapping}\n{}",
            tr!("alignment complete")
        )),
    ]);
    format!("{}\n{table}", tr!("Bytecode audit"))
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
    format!("{}\n{table}", tr!("Summary"))
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
                    tr!(
                        "Package: %{artifact}\nEnvironment: %{environment}\nTarget: %{target}\nReason: %{reason}\nRule: %{rule}\nImpact: %{impact} · Confidence: %{confidence} · Activation: %{activation} · Precision: %{precision}",
                        artifact = artifact,
                        environment = risk.environment_target,
                        target = format_target(&risk.target),
                        reason = risk.reason,
                        rule = risk.rule,
                        impact = severity_label(risk.severity),
                        confidence = confidence_label(risk.confidence),
                        activation = activation_label(risk.activation),
                        precision = precision_label(risk.precision),
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
                let mut details = tr!(
                    "Packages: %{left} ↔ %{right}\nTarget: %{target}\nReason: %{reason}\nRule: %{rule}\nImpact: %{impact} · Confidence: %{confidence} · Activation: %{activation} · Precision: %{precision}",
                    left = left,
                    right = right,
                    target = format_target(&risk.target),
                    reason = risk.reason,
                    rule = risk.rule,
                    impact = severity_label(risk.severity),
                    confidence = confidence_label(risk.confidence),
                    activation = activation_label(risk.activation),
                    precision = precision_label(risk.precision),
                );
                if !sources.is_empty() {
                    details.push_str(&tr!("\nSource: "));
                    details.push_str(&sources.into_iter().collect::<Vec<_>>().join(" × "));
                }
                (risk.risk_index, details)
            }
        };
        table.add_row([
            Cell::new(tr!(
                "#%{number}\nRISK %{risk}",
                number = index + 1,
                risk = risk_index
            )),
            Cell::new(details),
        ]);
    }

    format!(
        "{}\n{table}",
        tr!(
            "Risks (showing %{shown} of %{total})",
            shown = shown,
            total = total
        )
    )
}

fn precision_label(precision: orbit_core::AuditPrecision) -> std::borrow::Cow<'static, str> {
    match precision {
        orbit_core::AuditPrecision::Instruction => tr!("instruction"),
        orbit_core::AuditPrecision::Pattern => tr!("pattern"),
        orbit_core::AuditPrecision::Method => tr!("method"),
        orbit_core::AuditPrecision::Class => tr!("class"),
        orbit_core::AuditPrecision::Unknown => tr!("unknown"),
    }
}

fn format_target(target: &orbit_core::audit_model::Target) -> String {
    target.member.as_ref().map_or_else(
        || target.class.clone(),
        |member| format!("{}::{}{}", target.class, member.name, member.descriptor),
    )
}

fn namespace_label(namespace: orbit_core::AuditSymbolNamespace) -> std::borrow::Cow<'static, str> {
    match namespace {
        orbit_core::AuditSymbolNamespace::Runtime => tr!("runtime"),
        orbit_core::AuditSymbolNamespace::Official => tr!("official"),
        orbit_core::AuditSymbolNamespace::Intermediary => tr!("intermediary"),
        orbit_core::AuditSymbolNamespace::Srg => tr!("srg"),
        orbit_core::AuditSymbolNamespace::Named => tr!("named"),
        orbit_core::AuditSymbolNamespace::Identity => tr!("identity"),
        orbit_core::AuditSymbolNamespace::Unknown => tr!("unknown"),
    }
}

fn severity_label(severity: orbit_core::AuditSeverity) -> std::borrow::Cow<'static, str> {
    match severity {
        orbit_core::AuditSeverity::Low => tr!("LOW"),
        orbit_core::AuditSeverity::Medium => tr!("MEDIUM"),
        orbit_core::AuditSeverity::High => tr!("HIGH"),
        orbit_core::AuditSeverity::Critical => tr!("CRITICAL"),
    }
}

fn confidence_label(confidence: orbit_core::AuditConfidence) -> std::borrow::Cow<'static, str> {
    match confidence {
        orbit_core::AuditConfidence::Low => tr!("low"),
        orbit_core::AuditConfidence::Medium => tr!("medium"),
        orbit_core::AuditConfidence::High => tr!("high"),
        orbit_core::AuditConfidence::Exact => tr!("exact"),
    }
}

fn activation_label(activation: orbit_core::AuditActivation) -> std::borrow::Cow<'static, str> {
    match activation {
        orbit_core::AuditActivation::Definite => tr!("definite"),
        orbit_core::AuditActivation::Conditional => tr!("conditional"),
        orbit_core::AuditActivation::Candidate => tr!("candidate"),
        orbit_core::AuditActivation::Unknown => tr!("unknown"),
    }
}

fn mechanism_label(mechanism: orbit_core::AuditMechanism) -> std::borrow::Cow<'static, str> {
    match mechanism {
        orbit_core::AuditMechanism::Mixin => tr!("Mixin"),
        orbit_core::AuditMechanism::MixinExtras => tr!("MixinExtras"),
        orbit_core::AuditMechanism::ModLauncherTransformer => tr!("ModLauncher transformer"),
        orbit_core::AuditMechanism::JavaCoremod => tr!("Java coremod"),
        orbit_core::AuditMechanism::BinaryShape => tr!("binary shape"),
    }
}
