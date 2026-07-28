use super::super::*;

impl OrbitApp {
    pub(crate) fn show_audit(&mut self, ui: &mut egui::Ui) {
        theme::section_title(
            ui,
            "Compatibility audit",
            "Loader-aware bytecode and Mixin risk analysis",
        );
        if self.selected_instance().is_none() {
            ui.add_space(12.0);
            if installation_required_card(
                ui,
                "Choose an installation to audit",
                "The loader runtime and exact game JAR define the audit namespace.",
            ) {
                self.preferences.page = Page::Runtime;
            }
            return;
        }
        ui.horizontal(|ui| {
            if ui.add(theme::primary_button("Run audit")).clicked() {
                self.run_audit();
            }
            ui.label(RichText::new(tr!("Analysis is read-only.")).color(theme::muted()));
        });
        ui.add_space(12.0);
        if let Some(audit) = &self.audit {
            ui.columns(4, |columns| {
                metric_card(
                    &mut columns[0],
                    "Readiness",
                    audit.readiness.clone(),
                    "Loader backend",
                );
                metric_card(
                    &mut columns[1],
                    "Artifacts",
                    audit.artifacts.to_string(),
                    "Runtime inputs",
                );
                metric_card(
                    &mut columns[2],
                    "Warnings",
                    audit.warnings.to_string(),
                    "Coverage warnings",
                );
                metric_card(
                    &mut columns[3],
                    "Findings",
                    audit.findings.len().to_string(),
                    "Ranked risks",
                );
            });
            if audit.coverage_gaps > 0 {
                ui.label(
                    RichText::new(tr!(
                        "%{count} analysis coverage gap(s) require review.",
                        count = audit.coverage_gaps
                    ))
                    .color(theme::warning()),
                );
            }
            ui.add_space(12.0);
            ScrollArea::vertical().show(ui, |ui| {
                for finding in &audit.findings {
                    theme::card().show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let color = risk_color(finding.risk);
                            ui.label(
                                RichText::new(format!("{:02}", finding.risk))
                                    .size(22.0)
                                    .strong()
                                    .color(color),
                            );
                            ui.vertical(|ui| {
                                ui.label(RichText::new(&finding.packages).strong());
                                ui.label(RichText::new(&finding.rule).color(color));
                                ui.label(RichText::new(&finding.reason).color(theme::muted()));
                                ui.label(
                                    RichText::new(tr!(
                                        "%{severity} severity · %{confidence} confidence",
                                        severity = finding.severity,
                                        confidence = finding.confidence
                                    ))
                                    .size(12.0)
                                    .color(theme::muted()),
                                );
                            });
                        });
                    });
                    ui.add_space(8.0);
                }
            });
        } else {
            empty_state(
                ui,
                "No audit report",
                "Run a fresh analysis for the selected runtime.",
            );
        }
    }
}
