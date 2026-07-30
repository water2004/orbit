use gpui::{Context, IntoElement, ParentElement, Styled, Window, div};
use gpui_component::{
    ActiveTheme, Selectable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};

use super::super::OrbitApp;
use crate::app::components as ui;
use crate::assets::OrbitIcon;
use crate::model::AuditNotice;

const NOTICE_PREVIEW_LIMIT: usize = 8;

pub(super) fn render(
    app: &mut OrbitApp,
    _window: &mut Window,
    cx: &mut Context<OrbitApp>,
) -> impl IntoElement {
    let actions = Button::new("audit-run")
        .icon(OrbitIcon::Audit)
        .label(
            if app.audit.is_some() {
                tr!("Run a fresh check")
            } else {
                tr!("Run audit")
            }
            .into_owned(),
        )
        .primary()
        .on_click(cx.listener(|this, _, _, cx| {
            let filter = this.inputs.audit_filter.read(cx).value().trim().to_string();
            this.run_audit(None, filter);
            cx.notify();
        }));

    let filter_input = app.inputs.audit_filter.clone();
    let filter_read = filter_input.clone();
    let controls = ui::compact_card(cx).child(
        h_flex()
            .gap_2()
            .items_center()
            .child(ui::search_input(&filter_input).flex_1())
            .children(
                [(0, tr!("All risks")), (1, tr!("35+")), (2, tr!("70+"))]
                    .into_iter()
                    .map(|(index, label)| {
                        Button::new(("audit-risk", index))
                            .label(label.into_owned())
                            .selected(app.audit_min_risk == index)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.audit_min_risk = index;
                                cx.notify();
                            }))
                    }),
            )
            .child(
                Button::new("audit-export")
                    .label(tr!("Export full report…").into_owned())
                    .on_click(cx.listener(move |this, _, _, cx| {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("JSON", &["json"])
                            .set_file_name("orbit-audit.json")
                            .save_file()
                        {
                            let filter = filter_read.read(cx).value().trim().to_string();
                            this.run_audit(Some(path), filter);
                        }
                        cx.notify();
                    })),
            ),
    );

    let report = if app.selected_instance().is_none() {
        ui::themed_card(cx)
            .child(ui::empty_state(
                OrbitIcon::Runtime,
                tr!("No installation selected").into_owned(),
                tr!("Select an installation before evaluating its runtime bytecode.").into_owned(),
                None,
                cx,
            ))
            .into_any_element()
    } else if let Some(audit) = &app.audit {
        let mut body = v_flex()
            .gap_4()
            .child(
                h_flex()
                    .gap_3()
                    .flex_wrap()
                    .child(ui::metric(
                        tr!("Readiness").into_owned(),
                        audit.readiness.clone(),
                        tr!("Runtime namespace and inputs").into_owned(),
                        cx,
                    ))
                    .child(ui::metric(
                        tr!("Artifacts").into_owned(),
                        audit.artifacts.to_string(),
                        tr!("Logical runtime inputs").into_owned(),
                        cx,
                    ))
                    .child(ui::metric(
                        tr!("Warnings").into_owned(),
                        audit.warnings.len().to_string(),
                        tr!("Coverage warnings").into_owned(),
                        cx,
                    ))
                    .child(ui::metric(
                        tr!("Coverage gaps").into_owned(),
                        audit
                            .coverage_gaps
                            .iter()
                            .map(|gap| gap.count)
                            .sum::<usize>()
                            .to_string(),
                        tr!("Unresolved analysis scope").into_owned(),
                        cx,
                    )),
            )
            .child(runtime_profile(audit, cx))
            .child(ui::section_title(
                tr!("Ranked risks").into_owned(),
                tr!("Loader-aware bytecode and Mixin evidence").into_owned(),
                cx,
            ));
        if audit.findings.is_empty() {
            body = body.child(
                ui::themed_card(cx).child(ui::empty_state(
                    OrbitIcon::Audit,
                    tr!("No compatibility risks found").into_owned(),
                    tr!("The current audit did not find a reportable bytecode or Mixin risk.")
                        .into_owned(),
                    None,
                    cx,
                )),
            );
        } else {
            for finding in &audit.findings {
                let color = risk_color(finding.risk, cx);
                body = body.child(
                    ui::compact_card(cx)
                        .child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(ui::pill(
                                    format!("{} · {}", finding.severity, finding.risk),
                                    color.opacity(0.14),
                                    color,
                                ))
                                .child(div().font_semibold().child(finding.packages.clone()))
                                .child(ui::neutral_pill(finding.confidence.clone(), cx)),
                        )
                        .child(div().text_sm().font_medium().child(finding.rule.clone()))
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(finding.reason.clone()),
                        ),
                );
            }
        }
        if !audit.warnings.is_empty() || !audit.coverage_gaps.is_empty() {
            body = body.child(ui::section_title(
                tr!("Analysis limitations").into_owned(),
                tr!("Warnings describe damaged or ambiguous inputs; coverage gaps mark behavior the static analysis could not prove.").into_owned(),
                cx,
            ));
            for warning in audit.warnings.iter().take(NOTICE_PREVIEW_LIMIT) {
                body = body.child(notice_card(
                    warning,
                    tr!("Warning").as_ref(),
                    cx.theme().warning,
                    cx,
                ));
            }
            for gap in audit.coverage_gaps.iter().take(NOTICE_PREVIEW_LIMIT) {
                body = body.child(notice_card(
                    gap,
                    tr!("Coverage gap").as_ref(),
                    cx.theme().info,
                    cx,
                ));
            }
            let hidden = audit
                .warnings
                .len()
                .saturating_sub(NOTICE_PREVIEW_LIMIT)
                .saturating_add(
                    audit
                        .coverage_gaps
                        .len()
                        .saturating_sub(NOTICE_PREVIEW_LIMIT),
                );
            if hidden > 0 {
                body = body.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr!(
                            "%{count} more analysis limitation(s) are available in the full report.",
                            count = hidden
                        )),
                );
            }
        }
        body.into_any_element()
    } else {
        ui::themed_card(cx).child(ui::empty_state(
            OrbitIcon::Audit,
            tr!("No audit report yet").into_owned(),
            tr!("Run a fresh analysis against the selected runtime's exact Minecraft, Loader and mod package inputs.").into_owned(),
            None,
            cx,
        )).into_any_element()
    };

    let content = v_flex().gap_3().child(controls).child(report);

    ui::page(
        tr!("Compatibility audit").into_owned(),
        tr!("Loader-specific runtime namespaces, bytecode and active Mixin risk").into_owned(),
        actions,
        content,
        cx,
    )
}

fn runtime_profile(audit: &crate::model::AuditSummary, cx: &gpui::App) -> gpui::Div {
    let loader = audit
        .loader
        .as_deref()
        .map(presentation_label)
        .unwrap_or_else(|| tr!("Unknown").into_owned());
    let namespace = audit
        .runtime_namespace
        .as_deref()
        .map(presentation_label)
        .unwrap_or_else(|| tr!("Unknown").into_owned());
    let readiness = presentation_label(&audit.readiness);
    let mut capabilities = h_flex().gap_2().flex_wrap();
    if audit.capabilities.is_empty() {
        capabilities = capabilities.child(ui::neutral_pill(tr!("None").into_owned(), cx));
    } else {
        for capability in &audit.capabilities {
            capabilities = capabilities.child(ui::neutral_pill(presentation_label(capability), cx));
        }
    }

    let mut profile = ui::compact_card(cx)
        .child(
            h_flex()
                .gap_2()
                .items_center()
                .flex_wrap()
                .child(ui::pill(
                    readiness,
                    cx.theme().success.opacity(0.14),
                    cx.theme().success,
                ))
                .child(div().font_semibold().child(loader))
                .child(ui::neutral_pill(
                    tr!("Namespace: %{namespace}", namespace = namespace),
                    cx,
                )),
        )
        .child(
            h_flex()
                .gap_3()
                .items_center()
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr!("Loader capabilities").into_owned()),
                )
                .child(capabilities),
        );
    if !audit.readiness_message.is_empty() {
        profile = profile.child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(audit.readiness_message.clone()),
        );
    }
    profile
}

fn notice_card(
    notice: &AuditNotice,
    category: &str,
    color: gpui::Hsla,
    cx: &gpui::App,
) -> gpui::Div {
    let scope = notice.artifact.as_ref().map_or_else(
        || notice.scope.clone(),
        |artifact| format!("{artifact} · {}", notice.scope),
    );
    let mut header = h_flex()
        .gap_2()
        .items_center()
        .flex_wrap()
        .child(ui::pill(category.to_string(), color.opacity(0.14), color))
        .child(div().font_medium().child(presentation_label(&notice.kind)))
        .child(ui::neutral_pill(scope, cx));
    if notice.count > 1 {
        header = header.child(ui::neutral_pill(
            tr!("%{count} occurrence(s)", count = notice.count),
            cx,
        ));
    }
    ui::compact_card(cx).child(header).child(
        div()
            .text_sm()
            .text_color(cx.theme().muted_foreground)
            .child(notice.detail.clone()),
    )
}

fn presentation_label(code: &str) -> String {
    orbit_i18n::text(&crate::wire::humanize(code)).into_owned()
}

fn risk_color(risk: u8, cx: &gpui::App) -> gpui::Hsla {
    match risk {
        0..=34 => cx.theme().success,
        35..=69 => cx.theme().warning,
        _ => cx.theme().danger,
    }
}
