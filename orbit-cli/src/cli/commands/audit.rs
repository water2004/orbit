use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;

use super::CliContext;

pub async fn handle(
    min_risk: u8,
    fail_on_risk: Option<u8>,
    mod_filter: Option<String>,
    report_path: Option<PathBuf>,
    limit: usize,
    ctx: &CliContext,
) -> Result<()> {
    if limit == 0 {
        anyhow::bail!("{}", tr!("--limit must be at least 1"));
    }
    let instance_dir = ctx.instance_dir()?;
    let audit_progress = if ctx.output.ndjson_progress() {
        Some(crate::cli::output::ndjson_audit_reporter(
            ctx.command,
            ctx.machine_sequence.clone(),
        ))
    } else {
        crate::cli::progress::audit_reporter(ctx.quiet, ctx.runtime.config().ui.progress_bar)
    };
    let full_report = orbit_core::audit_instance_with_progress(&instance_dir, audit_progress)?;
    let selected_artifacts = selected_artifacts(&full_report, mod_filter.as_deref());
    if mod_filter.is_some() && selected_artifacts.as_ref().is_some_and(HashMap::is_empty) {
        anyhow::bail!(
            "{}",
            tr!(
                "No installed Mod artifact matches --mod '%{filter}'",
                filter = mod_filter.as_deref().unwrap_or_default()
            )
        );
    }
    let threshold_exceeded = fail_on_risk.is_some_and(|threshold| {
        exceeds_threshold(&full_report, threshold, selected_artifacts.as_ref())
    });
    if let Some(path) = &report_path {
        write_detailed_report(path, &full_report)?;
    }
    let mut report = full_report.clone();
    report.unary_risks.retain(|risk| {
        risk.risk_index >= min_risk
            && selected_artifacts
                .as_ref()
                .is_none_or(|selected| selected.contains_key(&risk.artifact_id))
    });
    report.risks.retain(|risk| {
        risk.risk_index >= min_risk
            && selected_artifacts.as_ref().is_none_or(|selected| {
                selected.contains_key(&risk.left_artifact)
                    || selected.contains_key(&risk.right_artifact)
            })
    });

    match ctx.output.format {
        crate::cli::output::OutputFormat::Text => {
            ctx.print_result(format_args!(
                "{}",
                crate::cli::output::audit_report(&report, limit)
            ));
            if let Some(path) = &report_path {
                ctx.print_result_line(format_args!(
                    "{}",
                    tr!("Detailed report written to: %{path}", path = path.display())
                ));
            }
        }
        crate::cli::output::OutputFormat::Json => {
            ctx.print_json("audit", &report);
        }
    }
    if threshold_exceeded {
        anyhow::bail!(
            "{}",
            tr!("Bytecode audit found risk at or above the --fail-on-risk threshold")
        );
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
    threshold: u8,
    selected: Option<&HashMap<String, ()>>,
) -> bool {
    report.unary_risks.iter().any(|risk| {
        risk.risk_index >= threshold
            && selected.is_none_or(|selected| selected.contains_key(&risk.artifact_id))
    }) || report.risks.iter().any(|risk| {
        risk.risk_index >= threshold
            && selected.is_none_or(|selected| {
                selected.contains_key(&risk.left_artifact)
                    || selected.contains_key(&risk.right_artifact)
            })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_risk_wording_does_not_claim_compatibility() {
        let report = empty_report();
        let text = crate::cli::output::audit_report(&report, 20);
        assert!(text.contains("No bytecode compatibility risks reached the current threshold."));
        assert!(!text.contains("All Mods are compatible"));
    }

    #[test]
    fn text_report_names_the_selected_runtime_capabilities() {
        let mut report = empty_report();
        report.readiness.capabilities =
            vec!["mixin".to_string(), "neoforge_class_processor".to_string()];

        let text = crate::cli::output::audit_report(&report, 20);

        assert!(text.contains("Audit capabilities"));
        assert!(text.contains("Mixin"));
        assert!(text.contains("NeoForge class processor"));
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

        let text = crate::cli::output::audit_report(&report, 20);

        assert!(text.contains("Structural compatibility risks: 1"));
        assert!(text.contains("Risks (showing 1 of 1)"));
        assert!(text.contains("Warnings"));
        assert!(!text.contains("Unresolved soft references"));
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

        let text = crate::cli::output::audit_report(&report, 1);

        assert!(text.contains("Risks (showing 1 of 2)"));
        assert_eq!(text.matches("Rule: test").count(), 1);
        assert!(
            text.lines().all(|line| line.chars().count() <= 120),
            "{text}"
        );
    }

    #[test]
    fn text_limit_is_global_across_unary_and_pairwise_risks() {
        let mut report = empty_report();
        let mut pair = sample_risk("pair");
        pair.rule = "pair-rule".to_string();
        report.risks.push(pair);
        report.unary_risks.push(orbit_core::AuditUnaryRisk {
            artifact_id: "a".to_string(),
            environment_target: "game runtime".to_string(),
            target: orbit_core::audit_model::Target::class("game/Foo"),
            rule: "unary-rule".to_string(),
            reason: "invalid transformation".to_string(),
            mutations: vec![orbit_core::AuditMutationKind::ReplaceMethodBody],
            evidence: Vec::new(),
            severity: orbit_core::AuditSeverity::High,
            confidence: orbit_core::AuditConfidence::Exact,
            precision: orbit_core::AuditPrecision::Method,
            risk_index: 90,
            activation: orbit_core::AuditActivation::Definite,
        });

        let text = crate::cli::output::audit_report(&report, 1);

        assert!(text.contains("Risks (showing 1 of 2)"));
        assert!(text.contains("Rule: unary-rule"));
        assert!(!text.contains("Rule: pair-rule"));
    }

    #[test]
    fn json_and_explicit_report_keep_complete_evidence() {
        let mut report = empty_report();
        report.risks.push(sample_risk("full structured evidence"));
        report
            .interactions
            .push(orbit_core::audit_model::BehavioralInteraction {
                left_artifact: "a".to_string(),
                right_artifact: "b".to_string(),
                target: orbit_core::audit_model::Target::method("game/Foo", "run", "()V"),
                kind: orbit_core::audit_model::BehavioralInteractionKind::OrderedValueDecorators,
                reason: "ordered but composable".to_string(),
                evidence: Vec::new(),
                confidence: orbit_core::AuditConfidence::Exact,
                activation: orbit_core::AuditActivation::Definite,
                order: orbit_core::AuditOrderAnalysis::LeftMustRunFirst,
            });
        report
            .coverage_gaps
            .push(orbit_core::audit_model::CoverageGap {
                artifact_id: Some("a".to_string()),
                scope: "example/Mixin".to_string(),
                kind: orbit_core::audit_model::CoverageGapKind::UnsupportedSelector,
                detail: "unsupported selector".to_string(),
                count: 1,
            });
        report
            .registered_mixin_configs
            .push(orbit_core::audit_model::RegisteredMixinConfig {
                artifact_id: "a".to_string(),
                config_path: "mixins.json".to_string(),
                side: orbit_core::audit_model::SideConstraint::Common,
                registration: orbit_core::audit_model::RegistrationSource::FabricMetadata,
                activation: orbit_core::audit_model::ConfigActivation::PluginControlled,
                required_mods: Vec::new(),
                behavior_version: None,
                parsed: Some(orbit_core::audit_model::ParsedMixinConfig {
                    required: false,
                    min_version: Some("0.8".to_string()),
                    compatibility_level: Some("JAVA_21".to_string()),
                    package: Some("example".to_string()),
                    plugin: Some("example.Plugin".to_string()),
                    refmap: Some("example.refmap.json".to_string()),
                    priority: 1000,
                    mixin_priority: 1000,
                    mixins: vec!["Mixin".to_string()],
                    client: Vec::new(),
                    server: Vec::new(),
                    default_require: 1,
                    default_group: "default".to_string(),
                    overwrite_require_annotations: true,
                }),
            });
        let json = serde_json::to_value(&report).unwrap();
        assert_eq!(
            json["risks"][0]["evidence"][0]["detail"],
            "full structured evidence"
        );
        assert_eq!(
            json["registered_mixin_configs"][0]["activation"],
            "plugin_controlled"
        );
        assert_eq!(
            json["registered_mixin_configs"][0]["parsed"]["plugin"],
            "example.Plugin"
        );
        assert!(json["transformations"].is_array());
        assert_eq!(json["risks"][0]["precision"], "instruction");
        assert_eq!(json["interactions"][0]["kind"], "ordered_value_decorators");
        assert_eq!(json["coverage_gaps"][0]["kind"], "unsupported_selector");
        assert!(json["warnings"].as_array().unwrap().is_empty());

        let path = std::env::temp_dir().join(format!(
            "orbit-audit-report-{}-{}.json",
            std::process::id(),
            report.schema_version
        ));
        write_detailed_report(&path, &report).unwrap();
        let written = std::fs::read_to_string(&path).unwrap();
        std::fs::remove_file(&path).unwrap();

        assert!(written.contains("full structured evidence"));
        assert!(written.contains("\"schema_version\": 5"));
    }

    #[test]
    fn json_keeps_fixed_schema_version() {
        let report = empty_report();
        let value = serde_json::to_value(report).unwrap();
        assert_eq!(value["schema_version"], 5);
        assert!(value.get("coverage").is_some());
        assert!(value.get("warnings").is_some());
    }

    #[test]
    fn fail_on_threshold_uses_effective_risk_index() {
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
            precision: orbit_core::AuditPrecision::Class,
            risk_index: 80,
            activation: orbit_core::AuditActivation::Candidate,
        });
        assert!(exceeds_threshold(&report, 80, None));
        assert!(!exceeds_threshold(&report, 81, None));
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
            precision: orbit_core::AuditPrecision::Instruction,
            risk_index: 75,
            activation: orbit_core::AuditActivation::Candidate,
        }
    }

    fn empty_report() -> orbit_core::AuditReport {
        orbit_core::AuditReport {
            schema_version: 5,
            environment: orbit_core::audit_model::AuditEnvironment {
                minecraft_version: "test".to_string(),
                loader: orbit_core::AuditLoaderFamily::Fabric,
                loader_version: "test".to_string(),
                physical_side: orbit_core::audit_model::PhysicalSide::Unknown,
                java_feature: 17,
            },
            readiness: orbit_core::AuditReadiness {
                status: orbit_core::AuditReadinessStatus::Ready,
                loader: Some(orbit_core::AuditLoaderFamily::Fabric),
                message: "ready".to_string(),
                capabilities: vec!["mixin".to_string()],
            },
            namespace: orbit_core::audit_model::NamespaceReport {
                runtime_namespace: Some(orbit_core::AuditSymbolNamespace::Identity),
                artifacts: Vec::new(),
                mapping_sources: Vec::new(),
                loader_units: Vec::new(),
                alignment: orbit_core::audit_model::NamespaceAlignment::Aligned {
                    runtime_namespace: orbit_core::AuditSymbolNamespace::Identity,
                },
                class_mapping_coverage: Default::default(),
                method_mapping_coverage: Default::default(),
                field_mapping_coverage: Default::default(),
                evidence: Vec::new(),
            },
            artifacts: Vec::new(),
            registered_mixin_configs: Vec::new(),
            registered_mixins: Vec::new(),
            transformations: Vec::new(),
            unary_risks: Vec::new(),
            risks: Vec::new(),
            interactions: Vec::new(),
            inactive_candidates: Vec::new(),
            coverage_gaps: Vec::new(),
            coverage: orbit_core::AuditCoverage::default(),
            warnings: Vec::new(),
        }
    }
}
