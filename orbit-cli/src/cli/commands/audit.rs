use std::collections::HashMap;

use anyhow::Result;

use crate::cli::{AuditFormat, AuditSeverity};

use super::CliContext;

pub async fn handle(
    format: AuditFormat,
    min_severity: AuditSeverity,
    fail_on: Option<AuditSeverity>,
    mod_filter: Option<String>,
    ctx: &CliContext,
) -> Result<()> {
    let instance_dir = ctx.instance_dir()?;
    let mut report = orbit_core::audit_instance(&instance_dir)?;
    let threshold = orbit_core::AuditSeverity::from(min_severity);
    let selected_artifacts = selected_artifacts(&report, mod_filter.as_deref());
    if mod_filter.is_some() && selected_artifacts.as_ref().is_some_and(HashMap::is_empty) {
        anyhow::bail!(
            "no installed Mod artifact matches --mod '{}'",
            mod_filter.as_deref().unwrap_or_default()
        );
    }
    let threshold_exceeded = fail_on.is_some_and(|severity| {
        exceeds_threshold(
            &report,
            orbit_core::AuditSeverity::from(severity),
            selected_artifacts.as_ref(),
        )
    });
    report.risks.retain(|risk| {
        risk.severity >= threshold
            && selected_artifacts.as_ref().is_none_or(|selected| {
                selected.contains_key(&risk.left_artifact)
                    || selected.contains_key(&risk.right_artifact)
            })
    });

    match format {
        AuditFormat::Text => print!("{}", render_text(&report)),
        AuditFormat::Json => println!("{}", serde_json::to_string_pretty(&report)?),
    }
    if threshold_exceeded {
        anyhow::bail!("bytecode audit found risk at or above the --fail-on threshold");
    }
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

fn render_text(report: &orbit_core::AuditReport) -> String {
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
        writeln!(output, "\nWarnings ({}):", report.warnings.len()).ok();
        for warning in &report.warnings {
            let artifact = warning
                .artifact_id
                .as_deref()
                .and_then(|id| labels.get(id).copied())
                .unwrap_or("runtime");
            writeln!(
                output,
                "  - [{artifact}] {}: {}",
                warning.scope, warning.message
            )
            .ok();
        }
    }

    if report.risks.is_empty() {
        writeln!(output, "\n未发现达到当前阈值的字节码兼容风险。").ok();
        return output;
    }

    writeln!(
        output,
        "\nPotential bytecode compatibility risks ({}):",
        report.risks.len()
    )
    .ok();
    for (index, risk) in report.risks.iter().enumerate() {
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
            "\n{}. {:?} / {:?} / risk index {} (not a probability)",
            index + 1,
            risk.severity,
            risk.confidence,
            risk.risk_index
        )
        .ok();
        writeln!(output, "   {left} ↔ {right}").ok();
        writeln!(
            output,
            "   target: {}{}",
            risk.target.class,
            format_member(&risk.target)
        )
        .ok();
        writeln!(
            output,
            "   rule: {} ({:?}, activation {:?})",
            risk.rule, risk.order, risk.activation
        )
        .ok();
        writeln!(output, "   reason: {}", risk.reason).ok();
        for evidence in &risk.evidence {
            writeln!(
                output,
                "   evidence: {} {}{} — {}",
                evidence.class,
                evidence.method.as_deref().unwrap_or(""),
                evidence
                    .instruction
                    .as_ref()
                    .map_or_else(String::new, |instruction| format!(
                        " [instruction {}, offset {:?}]",
                        instruction.stable_id, instruction.original_offset
                    )),
                evidence.detail
            )
            .ok();
        }
    }
    output
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
        let text = render_text(&report);
        assert!(text.contains("未发现达到当前阈值的字节码兼容风险。"));
        assert!(!text.contains("所有 Mod 均兼容"));
    }

    #[test]
    fn json_keeps_fixed_schema_version() {
        let report = empty_report();
        let value = serde_json::to_value(report).unwrap();
        assert_eq!(value["schema_version"], "1");
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

    fn empty_report() -> orbit_core::AuditReport {
        orbit_core::AuditReport {
            schema_version: "1".to_string(),
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
