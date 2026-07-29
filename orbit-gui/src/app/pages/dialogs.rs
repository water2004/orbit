use gpui::{Context, IntoElement, ParentElement, Styled, div, px};
use gpui_component::{
    ActiveTheme, Selectable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    scroll::ScrollableElement,
    switch::Switch,
    v_flex,
};

use super::super::{OrbitApp, Toast, ToastKind};
use crate::app::components as ui;
use crate::assets::OrbitIcon;

pub(super) fn render_runtime_rename(
    app: &OrbitApp,
    cx: &mut Context<OrbitApp>,
) -> impl IntoElement {
    let input = app.inputs.runtime_name.clone();
    let read = input.clone();
    ui::modal_backdrop(
        ui::modal(
            480.,
            v_flex()
                .gap_4()
                .child(
                    div()
                        .text_xl()
                        .font_semibold()
                        .child(tr!("Rename installation").into_owned()),
                )
                .child(ui::field(
                    tr!("Installation name").into_owned(),
                    tr!("The isolated instance directory is not moved").into_owned(),
                    &input,
                    cx,
                ))
                .child(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("runtime-rename-cancel")
                                .label(tr!("Cancel").into_owned())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.runtime_rename_open = false;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("runtime-rename-confirm")
                                .label(tr!("Rename").into_owned())
                                .primary()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let name = read.read(cx).value().trim().to_string();
                                    if !name.is_empty() {
                                        this.rename_runtime(name);
                                        this.runtime_rename_open = false;
                                    }
                                    cx.notify();
                                })),
                        ),
                ),
            cx,
        ),
        cx,
    )
}

pub(super) fn render_package_add(app: &OrbitApp, cx: &mut Context<OrbitApp>) -> impl IntoElement {
    let form = app.package_add.as_ref().expect("checked").clone();
    let version_input = app.inputs.add_version.clone();
    let version_read = version_input.clone();
    let project = form.project.clone();
    let environment = form.environment;
    let environments = [
        tr!("Follow JAR declaration").into_owned(),
        tr!("Client").into_owned(),
        tr!("Server").into_owned(),
        tr!("Both").into_owned(),
    ];
    ui::modal_backdrop(
        ui::modal(
            620.,
            v_flex()
                .gap_4()
                .child(
                    h_flex()
                        .justify_between()
                        .child(
                            v_flex()
                                .gap_1()
                                .child(
                                    div()
                                        .text_xl()
                                        .font_semibold()
                                        .child(tr!("Add %{project}", project = project.name)),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(format!(
                                            "{} · {}",
                                            project.platform, project.project_id
                                        )),
                                ),
                        )
                        .child(
                            Button::new("package-add-close")
                                .icon(OrbitIcon::Close)
                                .ghost()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.package_add = None;
                                    cx.notify();
                                })),
                        ),
                )
                .child(ui::field(
                    tr!("Version requirement").into_owned(),
                    tr!("Leave empty to accept any compatible JAR-declared version").into_owned(),
                    &version_input,
                    cx,
                ))
                .child(ui::section_title(
                    tr!("Environment filter").into_owned(),
                    tr!("This filters the managed package; it does not rewrite JAR metadata")
                        .into_owned(),
                    cx,
                ))
                .child(h_flex().gap_2().flex_wrap().children(
                    environments.into_iter().enumerate().map(|(index, label)| {
                        Button::new(("package-add-environment", index))
                            .label(label)
                            .selected(index == environment)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                if let Some(form) = &mut this.package_add {
                                    form.environment = index;
                                }
                                cx.notify();
                            }))
                    }),
                ))
                .child(
                    v_flex().gap_2().child(
                        Switch::new("package-add-optional")
                            .checked(form.optional)
                            .label(tr!("Optional package").into_owned())
                            .on_click(cx.listener(|this, checked, _, cx| {
                                if let Some(form) = &mut this.package_add {
                                    form.optional = *checked;
                                }
                                cx.notify();
                            })),
                    ),
                )
                .child(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("package-add-cancel")
                                .label(tr!("Cancel").into_owned())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.package_add = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("package-add-confirm")
                                .icon(OrbitIcon::Plus)
                                .label(tr!("Resolve and add").into_owned())
                                .primary()
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    let version = version_read.read(cx).value().trim().to_string();
                                    if let Some(form) = this.package_add.take() {
                                        this.add_search_result(
                                            &form.project,
                                            version,
                                            form.environment,
                                            form.optional,
                                        );
                                    }
                                    version_read
                                        .update(cx, |input, cx| input.set_value("", window, cx));
                                    cx.notify();
                                })),
                        ),
                ),
            cx,
        ),
        cx,
    )
}

pub(super) fn render_migration_review(
    app: &OrbitApp,
    cx: &mut Context<OrbitApp>,
) -> impl IntoElement {
    let review = app.migration_review.as_ref().expect("checked").clone();
    let plan = review.plan;
    let mut changes = v_flex().gap_2();
    if plan.changes.is_empty() {
        changes = changes.child(
            ui::compact_card(cx).child(tr!("The selected package set is unchanged.").into_owned()),
        );
    }
    for change in plan.changes {
        let version = match (&change.current_version, &change.selected_version) {
            (Some(current), Some(selected)) => format!("{current}  →  {selected}"),
            (None, Some(selected)) => selected.clone(),
            (Some(current), None) => current.clone(),
            (None, None) => change.selected_description.unwrap_or_default(),
        };
        changes = changes.child(
            ui::compact_card(cx).child(
                h_flex()
                    .gap_2()
                    .items_center()
                    .child(ui::neutral_pill(change.kind, cx))
                    .child(div().font_semibold().child(change.package))
                    .child(
                        div()
                            .ml_auto()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(version),
                    ),
            ),
        );
    }
    for warning in plan.warnings {
        changes = changes.child(
            ui::compact_card(cx).child(
                div()
                    .text_sm()
                    .text_color(cx.theme().warning)
                    .child(warning),
            ),
        );
    }
    for diagnostic in plan.diagnostics {
        changes = changes.child(
            ui::compact_card(cx)
                .child(
                    h_flex()
                        .gap_2()
                        .child(ui::neutral_pill(diagnostic.kind, cx))
                        .child(div().font_semibold().child(diagnostic.package)),
                )
                .children(diagnostic.facts.into_iter().map(|fact| {
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("• {fact}"))
                })),
        );
    }
    let summary = plan.summary;
    ui::modal_backdrop(
        ui::modal(
            780.,
            v_flex()
                .h(px(610.))
                .gap_4()
                .child(
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_xl()
                                .font_semibold()
                                .child(tr!("Migration compatibility review").into_owned()),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!(
                                    "{}  →  {} · {} {}",
                                    plan.source_mc_version,
                                    plan.target_mc_version,
                                    super::super::controller::title_case(&plan.target_loader),
                                    plan.target_loader_version
                                )),
                        ),
                )
                .child(
                    h_flex()
                        .gap_3()
                        .flex_wrap()
                        .child(ui::metric(
                            tr!("Selected packages").into_owned(),
                            summary.selected_packages.to_string(),
                            tr!("Resolved for the target runtime").into_owned(),
                            cx,
                        ))
                        .child(ui::metric(
                            tr!("Upgrades / downgrades").into_owned(),
                            format!("{} / {}", summary.upgrades, summary.downgrades),
                            tr!("Version changes").into_owned(),
                            cx,
                        ))
                        .child(ui::metric(
                            tr!("Installs / replacements / removals").into_owned(),
                            format!(
                                "{} / {} / {}",
                                summary.installs, summary.replacements, summary.removals
                            ),
                            tr!("Package-set changes").into_owned(),
                            cx,
                        )),
                )
                .child(
                    div()
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scrollbar()
                        .pr_1()
                        .child(changes),
                )
                .child(
                    h_flex()
                        .justify_between()
                        .items_center()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(format!(
                                    "{} · {}",
                                    tr!("The source installation is not modified."),
                                    plan.target_directory.display()
                                )),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    Button::new("migration-review-cancel")
                                        .label(tr!("Keep target without migration").into_owned())
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.migration_review = None;
                                            this.toast = Some(Toast {
                                                message: tr!("Migration was cancelled. The newly created target installation was kept.").into_owned(),
                                                kind: ToastKind::Warning,
                                            });
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Button::new("migration-review-apply")
                                        .label(tr!("Apply migration").into_owned())
                                        .primary()
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.apply_migration_review();
                                            cx.notify();
                                        })),
                                ),
                        ),
                ),
            cx,
        ),
        cx,
    )
}
