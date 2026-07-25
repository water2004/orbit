use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::cli::{AuditFormat, AuditSeverity};

use super::CliContext;

pub async fn handle(
    format: AuditFormat,
    min_severity: AuditSeverity,
    fail_on: Option<AuditSeverity>,
    mod_filter: Option<String>,
    report_path: Option<PathBuf>,
    limit: usize,
    ctx: &CliContext,
) -> Result<()> {
    if limit == 0 {
        anyhow::bail!("--limit must be at least 1");
    }
    let instance_dir = ctx.instance_dir()?;
    let full_report = orbit_core::audit_instance_with_progress(
        &instance_dir,
        crate::cli::progress::audit_reporter(ctx.quiet, &ctx.runtime.config().ui.progress_bar),
    )?;
    let threshold = orbit_core::AuditSeverity::from(min_severity);
    let selected_artifacts = selected_artifacts(&full_report, mod_filter.as_deref());
    if mod_filter.is_some() && selected_artifacts.as_ref().is_some_and(HashMap::is_empty) {
        anyhow::bail!(
            "no installed Mod artifact matches --mod '{}'",
            mod_filter.as_deref().unwrap_or_default()
        );
    }
    let threshold_exceeded = fail_on.is_some_and(|severity| {
        exceeds_threshold(
            &full_report,
            orbit_core::AuditSeverity::from(severity),
            selected_artifacts.as_ref(),
        )
    });
    if let Some(path) = &report_path {
        write_detailed_report(path, &full_report)?;
    }
    let mut report = full_report;
    report.risks.retain(|risk| {
        risk.severity >= threshold
            && selected_artifacts.as_ref().is_none_or(|selected| {
                selected.contains_key(&risk.left_artifact)
                    || selected.contains_key(&risk.right_artifact)
            })
    });

    match format {
        AuditFormat::Text => {
            print!("{}", render_text(&report, limit));
            if let Some(path) = &report_path {
                println!("Detailed report written to: {}", path.display());
            }
        }
        AuditFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
    }
    if format == AuditFormat::Json
        && let Some(path) = &report_path
    {
        eprintln!("Detailed report written to: {}", path.display());
    }
    if threshold_exceeded {
        anyhow::bail!("bytecode audit found risk at or above the --fail-on threshold");
    }
    Ok(())
}

fn write_detailed_report(path: &Path, report: &orbit_core::AuditReport) -> Result<()> {
    let mut json = serde_json::to_string_pretty(report)?;
    json.push('\n');
    std::fs::write(path, json)?;
    Ok(())
}

fn selected_artifacts(
    report: &orbit_core::AuditReport,
    filter: Option<&str>,
) -> Option<HashMap<String, ()>> {
    let filter = filter?.to_ascii_lowercase();
    Some(
        report
            .artifacts
            .iter()
            .filter(|artifact| {
                artifact.kind == orbit_core::AuditArtifactKind::Mod
                    && (artifact.id.to_ascii_lowercase().contains(&filter)
                        || artifact.display_name.to_ascii_lowercase().contains(&filter)
                        || std::path::Path::new(&artifact.path)
                            .file_name()
                            .and_then(|name| name.to_str())
                            .is_some_and(|name| name.to_ascii_lowercase().contains(&filter)))
            })
            .map(|artifact| (artifact.id.clone(), ()))
            .collect(),
    )
}

fn exceeds_threshold(
    report: &orbit_core::AuditReport,
    threshold: orbit_core::AuditSeverity,
    selected: Option<&HashMap<String, ()>>,
) -> bool {
    report.risks.iter().any(|risk| {
        risk.severity >= threshold
            && selected.is_none_or(|selected| {
                selected.contains_key(&risk.left_artifact)
                    || selected.contains_key(&risk.right_artifact)
            })
    })
}

fn render_text(report: &orbit_core::AuditReport, limit: usize) -> String {
    use std::fmt::Write;

    let labels = report
        .artifacts
        .iter()
        .map(|artifact| (artifact.id.as_str(), artifact.display_name.as_str()))
        .collect::<HashMap<_, _>>();
    let mut output = String::new();
    writeln!(
        output,
        "Bytecode audit: Minecraft {}, {} {}",
        report.environment.minecraft_version,
        report.environment.detected_loader,
        report.environment.loader_version
    )
    .ok();
    writeln!(
        output,
        "Readiness: {:?} — {}",
        report.readiness.status, report.readiness.message
    )
    .ok();
    writeln!(
        output,
        "Scanned {} JAR(s), parsed {}/{} class(es), discovered {} Mixin(s) and {} Transformer(s).",
        report.coverage.jars_scanned,
        report.coverage.classes_parsed,
        report.coverage.classes_discovered,
        report.coverage.mixins_discovered,
        report.coverage.transformers_discovered
    )
    .ok();
    writeln!(
        output,
        "Effect precision: {} instruction/pattern, {} method, {} class/unknown.",
        report.coverage.effects_instruction_precision,
        report.coverage.effects_method_precision,
        report.coverage.effects_class_precision
    )
    .ok();
    writeln!(
        output,
        "Transformer recovery: {} target(s), {} exact effect(s), {} partial, {} unknown.",
        report.coverage.transformer_targets_recovered,
        report.coverage.transformer_effects_recovered,
        report.coverage.transformer_effects_partial,
        report.coverage.transformer_effects_unknown
    )
    .ok();
    if !report.coverage.unsupported_mechanisms.is_empty() {
        writeln!(
            output,
            "Unsupported mechanisms: {}.",
            report.coverage.unsupported_mechanisms.join("; ")
        )
        .ok();
    }
    if !report.coverage.budget_exhaustions.is_empty() {
        writeln!(
            output,
            "Analysis budgets exhausted: {}.",
            report.coverage.budget_exhaustions.join("; ")
        )
        .ok();
    }

    if !report.warnings.is_empty() {
        let mut warning_counts = BTreeMap::new();
        for warning in &report.warnings {
            *warning_counts.entry(warning.kind).or_insert(0_usize) += 1;
        }
        writeln!(output, "\nWarnings ({}):", report.warnings.len()).ok();
        for (kind, count) in warning_counts {
            writeln!(output, "  {count:>4} {}", warning_label(kind)).ok();
        }
    }

    if report.risks.is_empty() {
        writeln!(output, "\n未发现达到当前阈值的字节码兼容风险。").ok();
        writeln!(
            output,
            "Use --format json or --report <path> for the complete structured report."
        )
        .ok();
        return output;
    }

    let severity_counts = [
        orbit_core::AuditSeverity::Critical,
        orbit_core::AuditSeverity::High,
        orbit_core::AuditSeverity::Medium,
        orbit_core::AuditSeverity::Low,
    ]
    .map(|severity| {
        (
            severity,
            report
                .risks
                .iter()
                .filter(|risk| risk.severity == severity)
                .count(),
        )
    });
    writeln!(
        output,
        "\nRisk distribution: {} critical, {} high, {} medium, {} low.",
        severity_counts[0].1, severity_counts[1].1, severity_counts[2].1, severity_counts[3].1,
    )
    .ok();
    let shown = report.risks.len().min(limit);
    writeln!(output, "Showing {shown} of {} risks.", report.risks.len()).ok();
    for risk in report.risks.iter().take(limit) {
        let left = labels
            .get(risk.left_artifact.as_str())
            .copied()
            .unwrap_or(&risk.left_artifact);
        let right = labels
            .get(risk.right_artifact.as_str())
            .copied()
            .unwrap_or(&risk.right_artifact);
        writeln!(
            output,
            "\n{:<8} {:>3}  {left} ↔ {right}",
            severity_label(risk.severity),
            risk.risk_index
        )
        .ok();
        writeln!(
            output,
            "  {}{}",
            risk.target.class,
            format_member(&risk.target)
        )
        .ok();
        writeln!(output, "  {}", risk.reason).ok();
        let mechanisms = risk
            .evidence
            .iter()
            .filter_map(|evidence| {
                evidence.mechanism.map(|mechanism| {
                    evidence.injector_kind.as_ref().map_or_else(
                        || format!("{mechanism:?}"),
                        |injector| format!("{mechanism:?} {injector}"),
                    )
                })
            })
            .collect::<BTreeSet<_>>();
        if !mechanisms.is_empty() {
            writeln!(
                output,
                "  source: {}",
                mechanisms.into_iter().collect::<Vec<_>>().join(" × ")
            )
            .ok();
        }
        writeln!(
            output,
            "  rule {}, confidence {:?}, activation {:?}",
            risk.rule, risk.confidence, risk.activation
        )
        .ok();
    }
    writeln!(
        output,
        "\nUse --format json or --report <path> for all evidence, selectors, warnings, and offsets."
    )
    .ok();
    output
}

fn severity_label(severity: orbit_core::AuditSeverity) -> &'static str {
    match severity {
        orbit_core::AuditSeverity::Low => "LOW",
        orbit_core::AuditSeverity::Medium => "MEDIUM",
        orbit_core::AuditSeverity::High => "HIGH",
        orbit_core::AuditSeverity::Critical => "CRITICAL",
    }
}

fn warning_label(kind: orbit_core::AuditWarningKind) -> &'static str {
    match kind {
        orbit_core::AuditWarningKind::UnresolvedSoftReference => "unresolved soft references",
        orbit_core::AuditWarningKind::AmbiguousSoftReference => "ambiguous soft references",
        orbit_core::AuditWarningKind::KnownUnsupportedInjectionPoint => {
            "known but unsupported injection points"
        }
        orbit_core::AuditWarningKind::CustomInjectionPoint => "custom injection points",
        orbit_core::AuditWarningKind::DamagedArtifact => "damaged artifacts",
        orbit_core::AuditWarningKind::DamagedClass => "damaged classes",
        orbit_core::AuditWarningKind::TransformerPartial => "partial transformer analyses",
        orbit_core::AuditWarningKind::UnsupportedMechanism => "unsupported mechanisms",
        orbit_core::AuditWarningKind::BudgetExhaustion => "analysis budget exhaustions",
        orbit_core::AuditWarningKind::Other => "other warnings",
    }
}

fn format_member(target: &orbit_core::audit_model::Target) -> String {
    target.member.as_ref().map_or_else(String::new, |member| {
        format!("::{}{}", member.name, member.descriptor)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_risk_wording_does_not_claim_compatibility() {
        let report = empty_report();
        let text = render_text(&report, 20);
        assert!(text.contains("未发现达到当前阈值的字节码兼容风险。"));
        assert!(!text.contains("所有 Mod 均兼容"));
    }

    #[test]
    fn default_text_is_bounded_and_omits_full_evidence_and_warning_details() {
        let mut report = empty_report();
        report.risks.push(sample_risk("secret evidence detail"));
        report.warnings.push(orbit_core::audit_model::Warning::new(
            Some("a".to_string()),
            "example/Mixin",
            orbit_core::AuditWarningKind::UnresolvedSoftReference,
            "secret warning detail",
        ));

        let text = render_text(&report, 20);

        assert!(text.contains("Risk distribution:"));
        assert!(text.contains("Showing 1 of 1 risks."));
        assert!(text.contains("1 unresolved soft references"));
        assert!(!text.contains("evidence:"));
        assert!(!text.contains("secret evidence detail"));
        assert!(!text.contains("secret warning detail"));
    }

    #[test]
    fn text_limit_reports_how_many_risks_are_hidden() {
        let mut report = empty_report();
        report.risks.push(sample_risk("first"));
        let mut second = sample_risk("second");
        second.left_artifact = "c".to_string();
        report.risks.push(second);

        let text = render_text(&report, 1);

        assert!(text.contains("Showing 1 of 2 risks."));
        assert_eq!(text.matches("rule test").count(), 1);
    }

    #[test]
    fn json_and_explicit_report_keep_complete_evidence() {
        let mut report = empty_report();
        report.risks.push(sample_risk("full structured evidence"));
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(
            json["risks"][0]["evidence"][0]["detail"],
            "full structured evidence"
        );

        let path = std::env::temp_dir().join(format!(
            "orbit-audit-report-{}-{}.json",
            std::process::id(),
            report.schema_version
        ));
        write_detailed_report(&path, &report).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert!(written.contains("full structured evidence"));
        assert!(written.contains("\"schema_version\": \"2\""));
    }

    #[test]
    fn json_keeps_fixed_schema_version() {
        let report = empty_report();
        let value = serde_json::to_value(report).unwrap();
        assert_eq!(value["schema_version"], "2");
        assert!(value.get("coverage").is_some());
        assert!(value.get("warnings").is_some());
    }

    #[test]
    fn fail_on_threshold_uses_severity_ordering() {
        let mut report = empty_report();
        report.risks.push(orbit_core::AuditRisk {
            left_artifact: "a".to_string(),
            right_artifact: "b".to_string(),
            target: orbit_core::audit_model::Target::class("game/Foo"),
            rule: "test".to_string(),
            reason: "test".to_string(),
            left_mutations: vec![orbit_core::AuditMutationKind::ReplaceInstruction],
            right_mutations: vec![orbit_core::AuditMutationKind::RemoveInstruction],
            evidence: Vec::new(),
            order: orbit_core::AuditOrderAnalysis::Exclusive,
            severity: orbit_core::AuditSeverity::High,
            confidence: orbit_core::AuditConfidence::Exact,
            risk_index: 80,
            activation: orbit_core::AuditActivation::Candidate,
        });
        assert!(exceeds_threshold(
            &report,
            orbit_core::AuditSeverity::High,
            None
        ));
        assert!(!exceeds_threshold(
            &report,
            orbit_core::AuditSeverity::Critical,
            None
        ));
    }

    #[test]
    fn mod_filter_does_not_select_loader_or_runtime_artifacts() {
        let mut report = empty_report();
        report
            .artifacts
            .push(orbit_core::audit_model::ArtifactReport {
                id: "loader:fabric".to_string(),
                display_name: "fabric loader".to_string(),
                path: "fabric-loader.jar".to_string(),
                kind: orbit_core::AuditArtifactKind::Loader,
                size: 1,
                sha256: "00".to_string(),
            });
        report
            .artifacts
            .push(orbit_core::audit_model::ArtifactReport {
                id: "mod:0:fabric-api.jar".to_string(),
                display_name: "fabric-api".to_string(),
                path: "mods/fabric-api.jar".to_string(),
                kind: orbit_core::AuditArtifactKind::Mod,
                size: 1,
                sha256: "00".to_string(),
            });

        let selected = selected_artifacts(&report, Some("fabric")).unwrap();

        assert_eq!(selected.len(), 1);
        assert!(selected.contains_key("mod:0:fabric-api.jar"));
    }

    fn sample_risk(detail: &str) -> orbit_core::AuditRisk {
        let mut evidence = orbit_core::audit_model::Evidence::new("a", "example/Mixin", detail);
        evidence.mechanism = Some(orbit_core::AuditMechanism::Mixin);
        evidence.injector_kind = Some("Redirect".to_string());
        orbit_core::AuditRisk {
            left_artifact: "a".to_string(),
            right_artifact: "b".to_string(),
            target: orbit_core::audit_model::Target::method("game/Foo", "run", "()V"),
            rule: "test".to_string(),
            reason: "short reason".to_string(),
            left_mutations: vec![orbit_core::AuditMutationKind::RedirectOperation],
            right_mutations: vec![orbit_core::AuditMutationKind::RemoveInstruction],
            evidence: vec![evidence],
            order: orbit_core::AuditOrderAnalysis::Exclusive,
            severity: orbit_core::AuditSeverity::High,
            confidence: orbit_core::AuditConfidence::Exact,
            risk_index: 75,
            activation: orbit_core::AuditActivation::Candidate,
        }
    }

    fn empty_report() -> orbit_core::AuditReport {
        orbit_core::AuditReport {
            schema_version: "2".to_string(),
            environment: orbit_core::audit_model::AuditEnvironment {
                minecraft_version: "test".to_string(),
                declared_loader: "fabric".to_string(),
                detected_loader: "fabric".to_string(),
                loader_version: "test".to_string(),
            },
            readiness: orbit_core::AuditReadiness {
                status: orbit_core::AuditReadinessStatus::Ready,
                loader: Some(orbit_core::AuditLoaderFamily::Fabric),
                message: "ready".to_string(),
                capabilities: vec!["mixin".to_string()],
            },
            artifacts: Vec::new(),
            risks: Vec::new(),
            coverage: orbit_core::AuditCoverage::default(),
            warnings: Vec::new(),
        }
    }
}
