use std::collections::{BTreeMap, BTreeSet, HashMap};

use comfy_table::Cell;

use super::output_table;

pub fn audit_report(report: &orbit_core::AuditReport, limit: usize) -> String {
    let mut sections = vec![environment_table(report), coverage_table(&report.coverage)];

    if !report.coverage_gaps.is_empty() {
        sections.push(coverage_gaps_table(report));
    }
    if !report.inactive_candidates.is_empty() {
        sections.push(inactive_candidates_table(report));
    }
    if !report.interactions.is_empty() {
        sections.push(interactions_table(report));
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
        sections.push(format!(
            "Structural compatibility risks: {}",
            report.risks.len()
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
        Cell::new(format!(
            "{} parse failures; {} budget degradations; {} instruction-resolution degradations",
            coverage.method_parse_failures,
            coverage.method_budget_degradations,
            coverage.instruction_resolution_degraded
        )),
    ]);
    table.add_row([
        Cell::new("Mixins"),
        Cell::new(format!(
            "{} registered / {} analyzed",
            coverage.mixins_registered, coverage.mixins_discovered
        )),
        Cell::new(format!(
            "{} inactive; {} instruction/pattern, {} method, {} class/unknown effects",
            coverage.inactive_mixins,
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

fn coverage_gaps_table(report: &orbit_core::AuditReport) -> String {
    let mut counts = BTreeMap::new();
    let mut total = 0_usize;
    for gap in &report.coverage_gaps {
        *counts.entry(gap.kind).or_insert(0_usize) += gap.count;
        total = total.saturating_add(gap.count);
    }
    let mut table = output_table(["Coverage gap", "Count"]);
    for (kind, count) in counts {
        table.add_row([Cell::new(coverage_gap_label(kind)), Cell::new(count)]);
    }
    format!("Coverage gaps ({total})\n{table}")
}

fn inactive_candidates_table(report: &orbit_core::AuditReport) -> String {
    let mut counts = BTreeMap::new();
    for candidate in &report.inactive_candidates {
        *counts.entry(candidate.kind).or_insert(0_usize) += 1;
    }
    let mut table = output_table(["Inactive candidate", "Count"]);
    for (kind, count) in counts {
        table.add_row([Cell::new(inactive_candidate_label(kind)), Cell::new(count)]);
    }
    format!(
        "Inactive candidates ({})\n{table}",
        report.inactive_candidates.len()
    )
}

fn interactions_table(report: &orbit_core::AuditReport) -> String {
    let mut counts = BTreeMap::new();
    for interaction in &report.interactions {
        *counts.entry(interaction.kind).or_insert(0_usize) += 1;
    }
    let mut table = output_table(["Behavioral interaction", "Count"]);
    for (kind, count) in counts {
        table.add_row([Cell::new(interaction_label(kind)), Cell::new(count)]);
    }
    format!(
        "Behavioral interactions ({})\n{table}",
        report.interactions.len()
    )
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

        table.add_row([
            Cell::new(format!("#{}\nRISK {}", index + 1, risk.risk_index)),
            Cell::new(details),
        ]);
    }

    format!("Risks (showing {shown} of {})\n{table}", report.risks.len())
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
        orbit_core::AuditWarningKind::MalformedConfig => "Malformed Mixin configs",
        orbit_core::AuditWarningKind::TransformerPartial => "Partial transformer analyses",
        orbit_core::AuditWarningKind::UnsupportedMechanism => "Unsupported mechanisms",
        orbit_core::AuditWarningKind::BudgetExhaustion => "Analysis budget exhaustions",
        orbit_core::AuditWarningKind::Other => "Other warnings",
    }
}

fn coverage_gap_label(kind: orbit_core::audit_model::CoverageGapKind) -> &'static str {
    use orbit_core::audit_model::CoverageGapKind;
    match kind {
        CoverageGapKind::UnsupportedSelector => "Unsupported selector syntax",
        CoverageGapKind::UnsupportedInjectionPoint => "Unsupported injection point",
        CoverageGapKind::UnresolvedLocalSelector => "Unresolved local selector",
        CoverageGapKind::DynamicMixinConfigRegistration => "Dynamic Mixin config registration",
        CoverageGapKind::PluginDecision => "Dynamic plugin decision",
        CoverageGapKind::PluginDynamicMixins => "Dynamic plugin Mixin list",
        CoverageGapKind::PluginClassMutation => "Plugin class mutation",
        CoverageGapKind::TransformerPartial => "Partial transformer analysis",
        CoverageGapKind::TransformerUnknown => "Unknown transformer effect",
        CoverageGapKind::BudgetExhaustion => "Analysis budget exhausted",
        CoverageGapKind::FutureClassfile => "Future ClassFile best effort",
        CoverageGapKind::PhysicalSideUnknown => "Unknown physical side",
        CoverageGapKind::UnsupportedMechanism => "Unsupported mechanism",
    }
}

fn inactive_candidate_label(kind: orbit_core::audit_model::InactiveCandidateKind) -> &'static str {
    use orbit_core::audit_model::InactiveCandidateKind;
    match kind {
        InactiveCandidateKind::UnregisteredConfig => "Unregistered Mixin config",
        InactiveCandidateKind::SideMismatch => "Physical side mismatch",
        InactiveCandidateKind::MissingRequiredMods => "Missing required Mod",
        InactiveCandidateKind::PluginRejected => "Plugin rejected",
        InactiveCandidateKind::MissingOptionalTarget => "Missing optional target",
        InactiveCandidateKind::PseudoTargetMissing => "Missing @Pseudo target",
        InactiveCandidateKind::UnregisteredTransformer => "Unregistered transformer",
    }
}

fn interaction_label(kind: orbit_core::audit_model::BehavioralInteractionKind) -> &'static str {
    use orbit_core::audit_model::BehavioralInteractionKind;
    match kind {
        BehavioralInteractionKind::OrderedValueDecorators => "Ordered value decorators",
        BehavioralInteractionKind::OrderedMethodContributions => "Ordered method contributions",
        BehavioralInteractionKind::OptionalInjectionAffected => "Optional injection affected",
        BehavioralInteractionKind::OrderDependentTransformation => "Order-dependent transformation",
    }
}
