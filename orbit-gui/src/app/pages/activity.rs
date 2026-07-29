use std::time::Duration;

use gpui::{
    Animation, AnimationExt, AnyElement, Context, InteractiveElement, IntoElement, ParentElement,
    StatefulInteractiveElement, Styled, Window, div, ease_in_out, prelude::FluentBuilder as _, px,
    relative,
};
use gpui_component::{
    ActiveTheme, Disableable, Icon, Selectable, StyledExt,
    animation::cubic_bezier,
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    progress::Progress,
    scroll::ScrollableElement,
    v_flex,
};
use orbit_machine_protocol::{InteractionKind, ProgressPhase};
use serde::Deserialize;

use super::super::{
    ACTIVITY_DRAWER_TRANSITION, OrbitApp, PackageEditorSection, PackagePolicyDraft,
    PackagePolicyMode, PackagePolicyOperator, TaskState, ToastKind, controller::human_bytes,
};
use crate::app::components as ui;
use crate::assets::OrbitIcon;
use crate::model::PackageChange;
use crate::string_rule::{StringOperationDraft, StringPredicate, StringSetOperator};

pub(in crate::app) fn render_strip(app: &OrbitApp, cx: &mut Context<OrbitApp>) -> impl IntoElement {
    let task = app
        .tasks
        .values()
        .rev()
        .find(|task| task.state == TaskState::Running)
        .or_else(|| app.tasks.values().next_back());
    let Some(task) = task else {
        return div().into_any_element();
    };
    let running = task.state == TaskState::Running;
    let task_id = task.id;
    let progress = progress_percent(task.completed, task.total);
    let completed = task.completed;
    let total = task.total;
    let status_line = single_line_summary(&task.status_line);
    v_flex()
        .relative()
        .h(px(54.))
        .flex_shrink_0()
        .border_t_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().group_box)
        .child(
            h_flex()
                .h(px(43.))
                .px_4()
                .gap_2()
                .items_center()
                .child(
                    div()
                        .size(px(7.))
                        .rounded_full()
                        .bg(task_state_color(task.state, cx)),
                )
                .child(
                    h_flex()
                        .min_w_0()
                        .flex_1()
                        .gap_2()
                        .child(
                            div()
                                .min_w_0()
                                .max_w(px(360.))
                                .truncate()
                                .font_medium()
                                .child(task.label.clone()),
                        )
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .truncate()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(status_line),
                        ),
                )
                .when(completed.is_some() || total.is_some(), |row| {
                    row.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(match (completed, total) {
                                (Some(done), Some(total))
                                    if task.phase == Some(ProgressPhase::Export) =>
                                {
                                    format!("{} / {}", human_bytes(done), human_bytes(total))
                                }
                                (Some(done), Some(total)) => format!("{done} / {total}"),
                                (Some(done), None) => done.to_string(),
                                _ => String::new(),
                            }),
                    )
                })
                .when(running, |row| {
                    row.child(
                        Button::new("strip-cancel")
                            .label(tr!("Cancel").into_owned())
                            .ghost()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.bridge.cancel(task_id);
                                if let Some(task) = this.tasks.get_mut(&task_id) {
                                    task.state = TaskState::Cancelled;
                                    task.status_line = tr!("Cancelling…").into_owned();
                                }
                                cx.notify();
                            })),
                    )
                })
                .child(
                    Button::new("strip-open")
                        .icon(OrbitIcon::Activity)
                        .label(tr!("Activity").into_owned())
                        .ghost()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.toggle_activity(cx);
                        })),
                ),
        )
        .child(match progress {
            Some(value) => Progress::new().h(px(6.)).value(value).into_any_element(),
            None if running => indeterminate(cx, 6.).into_any_element(),
            None => Progress::new()
                .h(px(6.))
                .value(if task.state == TaskState::Succeeded {
                    100.
                } else {
                    0.
                })
                .into_any_element(),
        })
        .into_any_element()
}

fn indeterminate(cx: &gpui::App, height: f32) -> impl IntoElement {
    div()
        .relative()
        .h(px(height))
        .w_full()
        .overflow_hidden()
        .bg(cx.theme().primary.opacity(0.18))
        .child(
            div()
                .absolute()
                .left_0()
                .top_0()
                .h_full()
                .w(relative(0.32))
                .rounded_full()
                .bg(cx.theme().primary)
                .with_animation(
                    "activity-indeterminate",
                    Animation::new(Duration::from_millis(1100))
                        .repeat()
                        .with_easing(ease_in_out),
                    |bar, delta| bar.left(relative(delta * 0.68)),
                ),
        )
}

pub(in crate::app) fn render_overlays(
    app: &mut OrbitApp,
    _window: &mut Window,
    cx: &mut Context<OrbitApp>,
) -> Vec<AnyElement> {
    let mut overlays = Vec::new();
    if app.activity_open || app.activity_closing {
        overlays.push(render_drawer_backdrop(app, cx).into_any_element());
        overlays.push(render_drawer(app, cx).into_any_element());
    }

    if app.interaction.is_some() {
        overlays.push(render_interaction(app, cx).into_any_element());
    } else if app.migration_review.is_some() {
        overlays.push(super::dialogs::render_migration_review(app, cx).into_any_element());
    } else if app.confirmation.is_some() {
        overlays.push(render_confirmation(app, cx).into_any_element());
    } else if app.microsoft_session.is_some() {
        overlays.push(render_microsoft(app, cx).into_any_element());
    } else if app.eula_document.is_some() {
        overlays.push(render_eula(app, cx).into_any_element());
    } else if app.package_add.is_some() {
        overlays.push(super::dialogs::render_package_add(app, cx).into_any_element());
    } else if app.runtime_rename_open {
        overlays.push(super::dialogs::render_runtime_rename(app, cx).into_any_element());
    } else if app.package_editor.is_some() {
        overlays.push(render_package_editor(app, cx).into_any_element());
    }
    if app.toast.is_some() {
        overlays.push(render_toast(app, cx).into_any_element());
    }
    overlays
}

fn render_drawer_backdrop(app: &OrbitApp, cx: &mut Context<OrbitApp>) -> impl IntoElement {
    let opening = app.activity_open;
    div()
        .id("activity-drawer-dismiss")
        .absolute()
        .inset_0()
        .bg(cx.theme().overlay.opacity(0.35))
        .on_click(cx.listener(|this, _, _, cx| {
            this.close_activity(cx);
        }))
        .with_animation(
            ("activity-drawer-backdrop", usize::from(opening)),
            Animation::new(ACTIVITY_DRAWER_TRANSITION).with_easing(cubic_bezier(0.2, 0.8, 0.2, 1.)),
            move |backdrop, delta| {
                let visibility = if opening { delta } else { 1. - delta };
                backdrop.opacity(visibility)
            },
        )
}

fn render_drawer(app: &OrbitApp, cx: &mut Context<OrbitApp>) -> impl IntoElement {
    let opening = app.activity_open;
    let mut history = v_flex().gap_2();
    for task in app.tasks.values().rev() {
        let task_id = task.id;
        let progress = progress_percent(task.completed, task.total);
        history = history.child(
            ui::compact_card(cx)
                .child(
                    h_flex()
                        .gap_2()
                        .items_center()
                        .child(
                            div()
                                .size(px(7.))
                                .rounded_full()
                                .bg(task_state_color(task.state, cx)),
                        )
                        .child(div().font_semibold().flex_1().child(task.label.clone()))
                        .child(ui::neutral_pill(task.command.clone(), cx))
                        .when(task.state == TaskState::Running, |row| {
                            row.child(
                                Button::new(("drawer-cancel", task_id as usize))
                                    .label(tr!("Cancel").into_owned())
                                    .ghost()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.bridge.cancel(task_id);
                                        cx.notify();
                                    })),
                            )
                        }),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(task.status_line.clone()),
                )
                .when_some(progress, |card, value| {
                    card.child(Progress::new().h(px(5.)).value(value))
                })
                .when(
                    task.state == TaskState::Running && progress.is_none(),
                    |card| card.child(indeterminate(cx, 5.)),
                )
                .when_some(task.error_message.clone(), |card, error| {
                    card.child(div().text_sm().text_color(cx.theme().danger).child(error))
                })
                .when(!task.log.is_empty(), |card| {
                    card.child(v_flex().gap_1().children(
                        task.log.iter().rev().take(4).rev().cloned().map(|line| {
                            div()
                                .font_family("monospace")
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(line)
                        }),
                    ))
                }),
        );
    }
    v_flex()
        .absolute()
        .top(px(58.))
        .right_0()
        .bottom(if app.tasks.is_empty() {
            px(0.)
        } else {
            px(54.)
        })
        .w(px(440.))
        .border_l_1()
        .border_color(cx.theme().border)
        .shadow_2xl()
        .bg(cx.theme().background)
        .child(
            h_flex()
                .h(px(54.))
                .px_4()
                .items_center()
                .justify_between()
                .border_b_1()
                .border_color(cx.theme().border)
                .child(
                    div()
                        .text_lg()
                        .font_semibold()
                        .child(tr!("Activity").into_owned()),
                )
                .child(
                    Button::new("drawer-close")
                        .icon(OrbitIcon::Close)
                        .ghost()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.close_activity(cx);
                        })),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .p_3()
                .child(history),
        )
        .with_animation(
            ("activity-drawer", usize::from(opening)),
            Animation::new(ACTIVITY_DRAWER_TRANSITION).with_easing(cubic_bezier(0.2, 0.8, 0.2, 1.)),
            move |drawer, delta| {
                let visibility = if opening { delta } else { 1. - delta };
                drawer
                    .right(px(-32.) + visibility * px(32.))
                    .opacity(0.7 + visibility * 0.3)
            },
        )
}

fn render_interaction(app: &OrbitApp, cx: &mut Context<OrbitApp>) -> impl IntoElement {
    let pending = app.interaction.as_ref().expect("checked").clone();
    let interaction_kind = pending.envelope.interaction;
    let parsed = pending
        .envelope
        .choices
        .iter()
        .map(|choice| interaction_package_actions(interaction_kind, &choice.data))
        .collect::<Vec<_>>();
    let common_actions = if interaction_kind == InteractionKind::Resolution {
        parsed
            .first()
            .and_then(|actions| actions.as_ref().ok())
            .map(|actions| {
                actions
                    .iter()
                    .filter(|action| !action.different)
                    .cloned()
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    let mut choices = v_flex().gap_2();
    if !common_actions.is_empty() {
        choices = choices
            .child(
                div()
                    .text_sm()
                    .font_semibold()
                    .child(tr!("Common package actions").into_owned()),
            )
            .child(ui::compact_card(cx).child(render_package_actions(&common_actions, false, cx)));
    }
    for (index, (choice, actions)) in pending
        .envelope
        .choices
        .iter()
        .cloned()
        .zip(parsed)
        .enumerate()
    {
        let choice_id = choice.id.clone();
        let invalid = actions.is_err();
        let actions = actions.unwrap_or_default();
        let visible_actions = if interaction_kind == InteractionKind::Resolution {
            actions
                .into_iter()
                .filter(|action| action.different)
                .collect::<Vec<_>>()
        } else {
            actions
        };
        let description = if interaction_kind == InteractionKind::Resolution {
            None
        } else {
            choice.description
        };
        let mut content = v_flex().w_full().min_w_0().gap_2().items_start().child(
            h_flex()
                .w_full()
                .min_w_0()
                .gap_2()
                .child(div().min_w_0().flex_1().font_semibold().child(choice.label))
                .when_some(description, |row, description| {
                    row.child(
                        div()
                            .flex_shrink_0()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(description),
                    )
                }),
        );
        if invalid {
            content = content.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().danger)
                    .child(tr!("The CLI returned invalid package-action data.").into_owned()),
            );
        } else if !visible_actions.is_empty() {
            content = content.child(render_package_actions(&visible_actions, true, cx));
        } else if interaction_kind == InteractionKind::Resolution {
            content = content.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr!("No package action differs in this option.").into_owned()),
            );
        }
        let card = div()
            .id(("interaction-choice", index))
            .w_full()
            .min_w_0()
            .p_3()
            .rounded_lg()
            .border_1()
            .border_color(cx.theme().border)
            .bg(cx.theme().group_box)
            .shadow_xs()
            .child(content)
            .when(!invalid, |card| {
                card.cursor_pointer()
                    .hover(|style| {
                        style
                            .bg(cx.theme().secondary)
                            .border_color(cx.theme().primary.opacity(0.45))
                    })
                    .on_click(cx.listener(move |this, _, _, cx| {
                        this.answer_interaction(Some(choice_id.clone()));
                        cx.notify();
                    }))
            })
            .when(invalid, |card| card.opacity(0.72));
        choices = choices.child(card);
    }
    ui::modal_backdrop(
        ui::modal(
            760.,
            v_flex()
                .h(px(540.))
                .max_h_full()
                .gap_3()
                .child(
                    div()
                        .text_xl()
                        .font_semibold()
                        .child(pending.envelope.prompt),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr!("◆ marks actions that differ between choices.").into_owned()),
                )
                .child(
                    div()
                        .id("interaction-choice-scroll")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scrollbar()
                        .pr_1()
                        .child(choices),
                )
                .when(pending.envelope.allow_cancel, |modal| {
                    modal.child(
                        h_flex().flex_shrink_0().justify_end().child(
                            Button::new("interaction-cancel")
                                .label(tr!("Cancel operation").into_owned())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.answer_interaction(None);
                                    cx.notify();
                                })),
                        ),
                    )
                }),
            cx,
        ),
        cx,
    )
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolutionChoiceData {
    changes: Vec<ResolutionChoiceAction>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct ResolutionChoiceAction {
    different: bool,
    change: PackageChange,
}

#[derive(Debug, Deserialize)]
struct ConfirmationChoiceData {
    #[serde(default)]
    changes: Vec<PackageChange>,
}

fn interaction_package_actions(
    kind: InteractionKind,
    data: &serde_json::Value,
) -> Result<Vec<ResolutionChoiceAction>, serde_json::Error> {
    match kind {
        InteractionKind::Resolution => {
            serde_json::from_value::<ResolutionChoiceData>(data.clone()).map(|data| data.changes)
        }
        InteractionKind::Confirmation => {
            serde_json::from_value::<ConfirmationChoiceData>(data.clone()).map(|data| {
                data.changes
                    .into_iter()
                    .map(|change| ResolutionChoiceAction {
                        different: false,
                        change,
                    })
                    .collect()
            })
        }
        InteractionKind::Package => Ok(Vec::new()),
    }
}

fn render_package_actions(
    actions: &[ResolutionChoiceAction],
    show_differences: bool,
    cx: &gpui::App,
) -> impl IntoElement {
    v_flex()
        .w_full()
        .gap_1p5()
        .children(actions.iter().map(|action| {
            let change = &action.change;
            let version = package_action_version(change);
            v_flex()
                .w_full()
                .min_w_0()
                .gap_1()
                .child(
                    h_flex()
                        .w_full()
                        .min_w_0()
                        .gap_2()
                        .items_center()
                        .child(div().w(px(12.)).text_color(cx.theme().primary).child(
                            if show_differences && action.different {
                                "◆"
                            } else {
                                ""
                            },
                        ))
                        .child(package_action_pill(&change.kind, cx))
                        .child(
                            div()
                                .min_w_0()
                                .flex_1()
                                .truncate()
                                .font_semibold()
                                .child(change.package.clone()),
                        )
                        .child(
                            div()
                                .flex_shrink_0()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(version),
                        ),
                )
                .when_some(change.selected_artifact.clone(), |row, filename| {
                    row.child(
                        h_flex()
                            .w_full()
                            .min_w_0()
                            .pl(px(20.))
                            .gap_2()
                            .items_center()
                            .child(ui::neutral_pill("JAR", cx))
                            .child(
                                div()
                                    .min_w_0()
                                    .flex_1()
                                    .truncate()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(filename),
                            ),
                    )
                })
                .when_some(change.selected_description.clone(), |row, description| {
                    row.child(
                        div()
                            .w_full()
                            .min_w_0()
                            .pl(px(20.))
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(description),
                    )
                })
        }))
}

fn package_action_pill(kind: &str, cx: &gpui::App) -> impl IntoElement {
    let (label, background, foreground) = match kind {
        "install" => (
            tr!("Install").into_owned(),
            cx.theme().success.opacity(0.14),
            cx.theme().success,
        ),
        "upgrade" => (
            tr!("Upgrade").into_owned(),
            cx.theme().primary.opacity(0.14),
            cx.theme().primary,
        ),
        "downgrade" => (
            tr!("Downgrade").into_owned(),
            cx.theme().warning.opacity(0.14),
            cx.theme().warning,
        ),
        "replace" => (
            tr!("Replace").into_owned(),
            cx.theme().warning.opacity(0.14),
            cx.theme().warning,
        ),
        "remove" => (
            tr!("Remove").into_owned(),
            cx.theme().danger.opacity(0.14),
            cx.theme().danger,
        ),
        "keep" => (
            tr!("Keep").into_owned(),
            cx.theme().secondary,
            cx.theme().secondary_foreground,
        ),
        other => (
            other.to_string(),
            cx.theme().secondary,
            cx.theme().secondary_foreground,
        ),
    };
    ui::pill(label, background, foreground)
}

fn package_action_version(change: &PackageChange) -> String {
    let absent = tr!("Not installed").into_owned();
    if change.kind == "keep" {
        return change.current_version.clone().unwrap_or(absent);
    }
    format!(
        "{}  →  {}",
        change
            .current_version
            .clone()
            .unwrap_or_else(|| absent.clone()),
        change.selected_version.clone().unwrap_or(absent)
    )
}

fn render_confirmation(app: &OrbitApp, cx: &mut Context<OrbitApp>) -> impl IntoElement {
    let confirmation = app.confirmation.as_ref().expect("checked").clone();
    let action = confirmation.action.clone();
    ui::modal_backdrop(
        ui::modal(
            480.,
            v_flex()
                .gap_4()
                .child(div().text_xl().font_semibold().child(confirmation.title))
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(confirmation.body),
                )
                .child(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("confirm-cancel")
                                .label(tr!("Cancel").into_owned())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.confirmation = None;
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("confirm-continue")
                                .label(tr!("Continue").into_owned())
                                .danger()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.confirmation = None;
                                    this.execute_confirmation(action.clone());
                                    cx.notify();
                                })),
                        ),
                ),
            cx,
        ),
        cx,
    )
}

fn render_microsoft(app: &OrbitApp, cx: &mut Context<OrbitApp>) -> impl IntoElement {
    let session = app.microsoft_session.as_ref().expect("checked").clone();
    let code = session
        .get("user_code")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("—")
        .to_string();
    let url = session
        .get("verification_uri")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("https://microsoft.com/devicelogin")
        .to_string();
    let session_id = session
        .get("login_session_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string);
    ui::modal_backdrop(
        ui::modal(
            500.,
            v_flex()
                .gap_4()
                .child(
                    div()
                        .text_xl()
                        .font_semibold()
                        .child(tr!("Microsoft sign in").into_owned()),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            tr!("Open the Microsoft device page and enter this code:").into_owned(),
                        ),
                )
                .child(
                    div()
                        .text_3xl()
                        .font_semibold()
                        .text_color(cx.theme().primary)
                        .child(code),
                )
                .child(div().text_sm().child(url))
                .child(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("microsoft-close")
                                .label(tr!("Close").into_owned())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.microsoft_session = None;
                                    cx.notify();
                                })),
                        )
                        .when_some(session_id, |row, session_id| {
                            row.child(
                                Button::new("microsoft-complete")
                                    .label(tr!("Complete sign in").into_owned())
                                    .primary()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.complete_microsoft_login(session_id.clone());
                                        cx.notify();
                                    })),
                            )
                        }),
                ),
            cx,
        ),
        cx,
    )
}

fn render_eula(app: &OrbitApp, cx: &mut Context<OrbitApp>) -> impl IntoElement {
    let document = app.eula_document.as_ref().expect("checked").clone();
    let text = document
        .get("text")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string();
    let url = document
        .get("url")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string();
    let digest = document
        .get("digest_sha256")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
        .to_string();
    let accept_digest = digest.clone();
    ui::modal_backdrop(
        ui::modal(
            760.,
            v_flex()
                .h(px(580.))
                .gap_3()
                .child(div().text_xl().font_semibold().child("Minecraft EULA"))
                .child(div().text_sm().text_color(cx.theme().primary).child(url))
                .child(div().text_xs().text_color(cx.theme().muted_foreground).child(format!("SHA-256 {digest}")))
                .child(div().flex_1().min_h_0().overflow_y_scrollbar().p_3().rounded_lg().bg(cx.theme().secondary).child(text))
                .child(
                    h_flex().justify_end().gap_2()
                        .child(Button::new("eula-close").label(tr!("Close without accepting").into_owned()).on_click(cx.listener(|this, _, _, cx| { this.eula_document = None; cx.notify(); })))
                        .child(Button::new("eula-accept").label(tr!("I agree").into_owned()).primary().disabled(accept_digest.is_empty()).on_click(cx.listener(move |this, _, _, cx| {
                            this.eula_document = None;
                            this.confirmation = Some(super::super::Confirmation {
                                title: tr!("Accept the Minecraft EULA?").into_owned(),
                                body: tr!("This records acceptance of exactly the displayed document digest for the selected server.").into_owned(),
                                action: super::super::ConfirmationAction::AcceptEula(accept_digest.clone()),
                            });
                            cx.notify();
                        }))),
                ),
            cx,
        ),
        cx,
    )
}

fn render_package_editor(app: &OrbitApp, cx: &mut Context<OrbitApp>) -> impl IntoElement {
    let editor = app.package_editor.as_ref().expect("checked").clone();
    let package_id = editor.package.mod_id.clone();
    let policy_package = package_id.clone();
    let body = match editor.section {
        PackageEditorSection::Numeric => render_numeric_policy(app, cx),
        PackageEditorSection::String => {
            render_string_policy(app, app.package_versions.as_ref(), cx)
        }
        PackageEditorSection::Settings => render_package_settings(app, cx),
    };
    let tabs = [
        (
            PackageEditorSection::Numeric,
            tr!("Numeric versions").into_owned(),
        ),
        (
            PackageEditorSection::String,
            tr!("String filter").into_owned(),
        ),
        (
            PackageEditorSection::Settings,
            tr!("Package settings").into_owned(),
        ),
    ];
    let mut navigation = h_flex().gap_1().p_1().rounded_lg().bg(cx.theme().secondary);
    for (index, (section, label)) in tabs.into_iter().enumerate() {
        navigation = navigation.child(
            Button::new(("package-editor-section", index))
                .label(label)
                .ghost()
                .selected(editor.section == section)
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(editor) = &mut this.package_editor {
                        editor.section = section;
                    }
                    cx.notify();
                })),
        );
    }
    let policy_footer = (editor.section != PackageEditorSection::Settings)
        .then(|| render_package_policy_footer(app, &policy_package, cx));

    ui::modal_backdrop(
        ui::modal(
            780.,
            v_flex()
                .h(px(640.))
                .max_h_full()
                .min_h_0()
                .gap_3()
                .child(
                    h_flex()
                        .flex_shrink_0()
                        .justify_between()
                        .items_center()
                        .child(
                            h_flex()
                                .gap_2()
                                .items_center()
                                .child(div().text_xl().font_semibold().child(package_id))
                                .child(ui::neutral_pill(editor.package.version.clone(), cx)),
                        )
                        .child(
                            Button::new("package-close")
                                .icon(OrbitIcon::Close)
                                .ghost()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.package_editor = None;
                                    cx.notify();
                                })),
                        ),
                )
                .child(navigation)
                .child(
                    div()
                        .id("package-editor-scroll")
                        .flex_1()
                        .min_h_0()
                        .overflow_y_scrollbar()
                        .pr_2()
                        .child(body),
                )
                .children(policy_footer),
            cx,
        ),
        cx,
    )
}

fn render_package_settings(app: &OrbitApp, cx: &mut Context<OrbitApp>) -> AnyElement {
    let editor = app.package_editor.as_ref().expect("checked").clone();
    let package_id = editor.package.mod_id.clone();
    let remote_package = package_id.clone();
    let purge_package = package_id.clone();
    let remote_input = app.inputs.remote_locator.clone();
    let remote_read = remote_input.clone();
    let providers = ["file", "modrinth", "curseforge"];
    let mut remotes = v_flex().gap_2();
    for (index, remote) in editor.package.remotes.iter().cloned().enumerate() {
        let package = package_id.clone();
        remotes = remotes.child(
            h_flex()
                .gap_2()
                .items_center()
                .child(div().flex_1().text_sm().child(remote))
                .child(
                    Button::new(("remote-remove", index))
                        .icon(OrbitIcon::Trash)
                        .ghost()
                        .disabled(editor.package.remotes.len() <= 1)
                        .tooltip(
                            if editor.package.remotes.len() <= 1 {
                                tr!("The last remote cannot be removed")
                            } else {
                                tr!("Remove remote")
                            }
                            .into_owned(),
                        )
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.remove_package_remote(&package, index);
                            this.package_editor = None;
                            cx.notify();
                        })),
                ),
        );
    }
    let environment = editor.environment.clone();
    v_flex()
        .gap_5()
        .pb_1()
        .child(ui::section_title(tr!("Environment").into_owned(), tr!("Auto follows the JAR declaration; loaders without a declaration default to both").into_owned(), cx))
        .child(
            h_flex().gap_2().children(["auto", "client", "server", "both"].into_iter().enumerate().map(|(index, value)| {
                let package = package_id.clone();
                Button::new(("package-env", index))
                    .label(title_environment(value))
                    .selected(environment == value)
                    .on_click(cx.listener(move |this, _, _, cx| { this.set_package_environment(&package, value); this.package_editor = None; cx.notify(); }))
            })),
        )
        .child(ui::section_title(tr!("Remotes").into_owned(), tr!("All sources are hash-deduplicated and analyzed equally").into_owned(), cx))
        .child(remotes)
        .child(
            h_flex().gap_2().children(providers.into_iter().enumerate().map(|(index, provider)| {
                Button::new(("remote-provider", index))
                    .label(provider)
                    .selected(editor.remote_provider == index)
                    .on_click(cx.listener(move |this, _, _, cx| { if let Some(editor) = &mut this.package_editor { editor.remote_provider = index; } cx.notify(); }))
            })),
        )
        .child(
            h_flex()
                .gap_2()
                .child(Input::new(&remote_input).flex_1())
                .child(Button::new("remote-add").icon(OrbitIcon::Plus).label(tr!("Add remote").into_owned()).primary().on_click(cx.listener(move |this, _, window, cx| {
                    let locator = remote_read.read(cx).value().trim().to_string();
                    if !locator.is_empty() {
                        let provider = this.package_editor.as_ref().map(|item| providers[item.remote_provider]).unwrap_or("file");
                        this.add_package_remote(&remote_package, provider, &locator);
                        remote_read.update(cx, |state, cx| state.set_value("", window, cx));
                        this.package_editor = None;
                    }
                    cx.notify();
                }))),
        )
        .child(ui::divider(cx))
        .child(
            h_flex()
                .justify_between()
                .gap_3()
                .child(
                    v_flex()
                        .gap_1()
                        .child(div().text_sm().font_semibold().child(tr!("Remove package data").into_owned()))
                        .child(div().text_xs().text_color(cx.theme().muted_foreground).child(tr!("Purge removes the package and presents matching configuration files before deletion.").into_owned())),
                )
                .child(
                    Button::new("package-purge")
                        .icon(OrbitIcon::Trash)
                        .label(tr!("Purge…").into_owned())
                        .danger()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.package_editor = None;
                            this.confirmation = Some(super::super::Confirmation {
                                title: tr!("Purge %{package}?", package = purge_package),
                                body: tr!("Orbit will first show the exact package and matching configuration files. Nothing is deleted until you confirm that plan.").into_owned(),
                                action: super::super::ConfirmationAction::PurgePackage(purge_package.clone()),
                            });
                            cx.notify();
                        })),
                ),
        )
        .into_any_element()
}

fn render_numeric_policy(app: &OrbitApp, cx: &mut Context<OrbitApp>) -> AnyElement {
    let draft = app
        .package_editor
        .as_ref()
        .expect("package editor checked")
        .policy
        .clone();
    let versions = app.package_versions.as_ref();
    let default_version = versions.and_then(|versions| {
        versions
            .candidates
            .iter()
            .find(|candidate| candidate.selected)
            .and_then(|candidate| candidate.numeric_core.clone())
            .or_else(|| {
                versions
                    .candidates
                    .iter()
                    .find_map(|candidate| candidate.numeric_core.clone())
            })
    });
    let range_lower = versions
        .into_iter()
        .flat_map(|versions| versions.candidates.iter().rev())
        .find_map(|item| item.numeric_core.clone());
    let range_upper = versions
        .into_iter()
        .flat_map(|versions| versions.candidates.iter())
        .find_map(|item| item.numeric_core.clone());
    let modes = [
        (PackagePolicyMode::Any, tr!("Any compatible").into_owned()),
        (
            PackagePolicyMode::Comparison,
            tr!("One boundary").into_owned(),
        ),
        (PackagePolicyMode::Range, tr!("Version range").into_owned()),
    ];
    let mut mode_controls = h_flex().gap_2();
    for (index, (mode, label)) in modes.into_iter().enumerate() {
        let default_version = default_version.clone();
        let lower = range_lower.clone();
        let upper = range_upper.clone();
        mode_controls = mode_controls.child(
            Button::new(("package-policy-mode", index))
                .label(label)
                .selected(draft.mode == mode)
                .disabled(versions.is_none())
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(editor) = &mut this.package_editor {
                        editor.policy.select_mode(mode, default_version.as_deref());
                        if mode == PackagePolicyMode::Range {
                            if editor.policy.lower.is_none() {
                                editor.policy.lower.clone_from(&lower);
                            }
                            if editor.policy.upper.is_none() {
                                editor.policy.upper.clone_from(&upper);
                            }
                        }
                    }
                    cx.notify();
                })),
        );
    }

    let mut builder = v_flex()
        .gap_3()
        .child(ui::section_title(
            tr!("Numeric version constraint").into_owned(),
            tr!("Choose a numeric core boundary without mixing in full-string filtering")
                .into_owned(),
            cx,
        ))
        .child(mode_controls)
        .when(draft.mode == PackagePolicyMode::Comparison, |content| {
            let operators = [
                (PackagePolicyOperator::Exact, tr!("Exactly =").into_owned()),
                (
                    PackagePolicyOperator::GreaterThan,
                    tr!("Newer than >").into_owned(),
                ),
                (
                    PackagePolicyOperator::AtLeast,
                    tr!("At least ≥").into_owned(),
                ),
                (
                    PackagePolicyOperator::LessThan,
                    tr!("Older than <").into_owned(),
                ),
                (PackagePolicyOperator::AtMost, tr!("At most ≤").into_owned()),
            ];
            content.child(
                h_flex()
                    .gap_2()
                    .flex_wrap()
                    .children(operators.into_iter().enumerate().map(
                        |(index, (operator, label))| {
                            Button::new(("package-policy-operator", index))
                                .label(label)
                                .selected(draft.operator == operator)
                                .disabled(versions.is_none())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(editor) = &mut this.package_editor {
                                        editor.policy.operator = operator;
                                    }
                                    cx.notify();
                                }))
                        },
                    )),
            )
        })
        .when(draft.mode == PackagePolicyMode::Range, |content| {
            content.child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("package-range-lower-inclusion")
                            .label(if draft.include_lower {
                                tr!("Include lower").into_owned()
                            } else {
                                tr!("Exclude lower").into_owned()
                            })
                            .selected(draft.include_lower)
                            .disabled(versions.is_none())
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(editor) = &mut this.package_editor {
                                    editor.policy.include_lower = !editor.policy.include_lower;
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("package-range-upper-inclusion")
                            .label(if draft.include_upper {
                                tr!("Include upper").into_owned()
                            } else {
                                tr!("Exclude upper").into_owned()
                            })
                            .selected(draft.include_upper)
                            .disabled(versions.is_none())
                            .on_click(cx.listener(|this, _, _, cx| {
                                if let Some(editor) = &mut this.package_editor {
                                    editor.policy.include_upper = !editor.policy.include_upper;
                                }
                                cx.notify();
                            })),
                    ),
            )
        })
        .child(ui::section_title(
            tr!("Available versions").into_owned(),
            versions
                .map(|versions| {
                    tr!(
                        "JAR-declared versions from all remotes · selected %{selected}",
                        selected = versions
                            .selected_version
                            .clone()
                            .unwrap_or_else(|| tr!("none").into_owned())
                    )
                })
                .unwrap_or_else(|| {
                    tr!("Downloading and inspecting configured remotes…").into_owned()
                }),
            cx,
        ));

    let mut version_list = v_flex();
    if let Some(versions) = versions {
        for (index, candidate) in versions.candidates.iter().enumerate() {
            let comparison_version = candidate.numeric_core.clone();
            let lower_version = candidate.numeric_core.clone();
            let upper_version = candidate.numeric_core.clone();
            let is_comparison = draft.version.as_deref() == candidate.numeric_core.as_deref();
            let is_lower = draft.lower.as_deref() == candidate.numeric_core.as_deref();
            let is_upper = draft.upper.as_deref() == candidate.numeric_core.as_deref();
            version_list = version_list.child(
                h_flex()
                    .id(("package-version", index))
                    .px_3()
                    .py_2()
                    .gap_2()
                    .items_center()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        v_flex()
                            .min_w_0()
                            .flex_1()
                            .gap_1()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(div().font_medium().child(candidate.version.clone()))
                                    .child(if candidate.numeric_filterable {
                                        ui::neutral_pill(
                                            candidate
                                                .numeric_core
                                                .as_ref()
                                                .map(|numeric| {
                                                    tr!("numeric %{numeric}", numeric = numeric)
                                                })
                                                .unwrap_or_else(|| {
                                                    tr!("No numeric core").into_owned()
                                                }),
                                            cx,
                                        )
                                        .max_w(px(220.))
                                        .truncate()
                                    } else {
                                        ui::pill(
                                            tr!("No numeric core").into_owned(),
                                            cx.theme().warning.opacity(0.13),
                                            cx.theme().warning,
                                        )
                                    })
                                    .child(if candidate.matches_constraint {
                                        ui::pill(
                                            tr!("Allowed").into_owned(),
                                            cx.theme().success.opacity(0.13),
                                            cx.theme().success,
                                        )
                                    } else {
                                        ui::neutral_pill(tr!("Excluded").into_owned(), cx)
                                    })
                                    .when(candidate.selected, |row| {
                                        row.child(ui::neutral_pill(
                                            tr!("Installed").into_owned(),
                                            cx,
                                        ))
                                    })
                                    .when(is_comparison, |row| {
                                        row.child(ui::neutral_pill(
                                            tr!("Boundary").into_owned(),
                                            cx,
                                        ))
                                    })
                                    .when(is_lower, |row| {
                                        row.child(ui::neutral_pill(tr!("Lower").into_owned(), cx))
                                    })
                                    .when(is_upper, |row| {
                                        row.child(ui::neutral_pill(tr!("Upper").into_owned(), cx))
                                    }),
                            )
                            .child(
                                div()
                                    .text_xs()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(match &candidate.numeric_error {
                                        Some(_) => format!(
                                            "{} · {} · {}",
                                            candidate.sources.join(", "),
                                            candidate.details,
                                            tr!("No Loader-recognized numeric core; only the string rule applies")
                                        ),
                                        None => format!(
                                            "{} · {}",
                                            candidate.sources.join(", "),
                                            candidate.details
                                        ),
                                    }),
                            ),
                    )
                    .when(draft.mode == PackagePolicyMode::Comparison, |row| {
                        row.child(
                            Button::new(("choose-version-boundary", index))
                                .label(tr!("Boundary").into_owned())
                                .selected(is_comparison)
                                .disabled(comparison_version.is_none())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    if let Some(editor) = &mut this.package_editor
                                        && let Some(version) = &comparison_version
                                    {
                                        editor.policy.version = Some(version.clone());
                                    }
                                    cx.notify();
                                })),
                        )
                    })
                    .when(draft.mode == PackagePolicyMode::Range, |row| {
                        row.child(
                            h_flex()
                                .gap_1()
                                .child(
                                    Button::new(("choose-version-lower", index))
                                        .label(tr!("Lower").into_owned())
                                        .selected(is_lower)
                                        .disabled(lower_version.is_none())
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if let Some(editor) = &mut this.package_editor
                                                && let Some(version) = &lower_version
                                            {
                                                editor.policy.lower = Some(version.clone());
                                            }
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Button::new(("choose-version-upper", index))
                                        .label(tr!("Upper").into_owned())
                                        .selected(is_upper)
                                        .disabled(upper_version.is_none())
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if let Some(editor) = &mut this.package_editor
                                                && let Some(version) = &upper_version
                                            {
                                                editor.policy.upper = Some(version.clone());
                                            }
                                            cx.notify();
                                        })),
                                ),
                        )
                    }),
            );
        }
    }
    builder = builder.child(ui::themed_card(cx).p_0().gap_0().child(version_list));
    builder.into_any_element()
}

fn numeric_policy_summary(draft: &PackagePolicyDraft) -> String {
    match draft.mode {
        PackagePolicyMode::Any => tr!("Every loader-compatible version is eligible").into_owned(),
        PackagePolicyMode::Comparison => draft
            .version
            .as_ref()
            .map(|version| format!("{} {version}", draft.operator.symbol()))
            .unwrap_or_else(|| tr!("Choose a boundary version").into_owned()),
        PackagePolicyMode::Range => match (&draft.lower, &draft.upper) {
            (Some(lower), Some(upper)) => format!(
                "{}{lower}, {upper}{}",
                if draft.include_lower { '[' } else { '(' },
                if draft.include_upper { ']' } else { ')' }
            ),
            _ => tr!("Choose lower and upper versions").into_owned(),
        },
    }
}

fn render_package_policy_footer(
    app: &OrbitApp,
    package_id: &str,
    cx: &mut Context<OrbitApp>,
) -> AnyElement {
    let draft = app
        .package_editor
        .as_ref()
        .expect("package editor checked")
        .policy
        .clone();
    let numeric = numeric_policy_summary(&draft);
    let string = if draft.string.operations.is_empty() {
        tr!("No string operations").into_owned()
    } else {
        tr!(
            "%{count} string operation(s)",
            count = draft.string.operations.len()
        )
    };
    let package = package_id.to_string();
    h_flex()
        .flex_shrink_0()
        .justify_between()
        .gap_3()
        .pt_3()
        .border_t_1()
        .border_color(cx.theme().border)
        .child(
            v_flex()
                .min_w_0()
                .gap_1()
                .child(
                    div()
                        .text_sm()
                        .font_semibold()
                        .child(tr!("Combined version policy").into_owned()),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr!(
                            "Numeric: %{numeric} · String: %{string}",
                            numeric = numeric,
                            string = string
                        )),
                )
                .when_some(draft.replaced_custom.clone(), |column, requirement| {
                    column.child(div().text_xs().text_color(cx.theme().warning).child(tr!(
                        "The existing custom policy '%{policy}' will be replaced",
                        policy = requirement
                    )))
                }),
        )
        .child(
            Button::new("package-policy-apply")
                .label(tr!("Apply policy").into_owned())
                .primary()
                .disabled(app.package_versions.is_none() || draft.command_args().is_none())
                .on_click(cx.listener(move |this, _, _, cx| {
                    if let Some(arguments) = this
                        .package_editor
                        .as_ref()
                        .and_then(|editor| editor.policy.command_args())
                    {
                        this.apply_package_policy(&package, arguments);
                        this.package_editor = None;
                    }
                    cx.notify();
                })),
        )
        .into_any_element()
}

fn string_set_operator_label(operator: StringSetOperator) -> String {
    match operator {
        StringSetOperator::Intersect => tr!("Intersect").into_owned(),
        StringSetOperator::Union => tr!("Union").into_owned(),
        StringSetOperator::Complement => tr!("Complement").into_owned(),
    }
}

fn string_condition_label(operation: &StringOperationDraft) -> String {
    if operation.operator == StringSetOperator::Complement {
        return tr!("Invert the current result set").into_owned();
    }
    let value = operation.value.clone().unwrap_or_default();
    let condition = match operation.predicate {
        StringPredicate::Empty => tr!("Version string is empty").into_owned(),
        StringPredicate::Present => tr!("Version string is present").into_owned(),
        StringPredicate::Equals => tr!("Equals '%{value}'", value = value),
        StringPredicate::Contains => tr!("Contains '%{value}'", value = value),
        StringPredicate::StartsWith => tr!("Starts with '%{value}'", value = value),
        StringPredicate::EndsWith => tr!("Ends with '%{value}'", value = value),
    };
    if operation.negated {
        tr!("Not: %{condition}", condition = condition)
    } else {
        condition
    }
}

fn render_string_policy(
    app: &OrbitApp,
    versions: Option<&crate::model::PackageVersions>,
    cx: &mut Context<OrbitApp>,
) -> AnyElement {
    let draft = app
        .package_editor
        .as_ref()
        .expect("package editor checked")
        .policy
        .clone();
    let input = app.inputs.string_value.clone();
    let input_read = input.clone();
    let mut operations = v_flex().gap_2();
    for (index, operation) in draft.string.operations.iter().cloned().enumerate() {
        let edit_operation = operation.clone();
        let edit_value = edit_operation.value.clone().unwrap_or_default();
        let edit_input = input.clone();
        let operator_label = string_set_operator_label(operation.operator);
        let condition_label = string_condition_label(&operation);
        operations = operations.child(
            h_flex()
                .id(("string-operation", index))
                .gap_2()
                .items_center()
                .px_3()
                .py_2()
                .rounded_lg()
                .border_1()
                .border_color(cx.theme().border)
                .child(
                    div()
                        .w(px(24.))
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!("{}", index + 1)),
                )
                .child(ui::neutral_pill(operator_label, cx))
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .text_sm()
                        .font_medium()
                        .child(condition_label),
                )
                .when(operation.needs_value(), |row| {
                    row.child(ui::neutral_pill(
                        if operation.case_sensitive {
                            tr!("Case-sensitive").into_owned()
                        } else {
                            tr!("Ignore case").into_owned()
                        },
                        cx,
                    ))
                })
                .child(
                    Button::new(("string-up", index))
                        .label("↑")
                        .ghost()
                        .disabled(index == 0)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if index > 0
                                && let Some(editor) = &mut this.package_editor
                            {
                                editor.policy.string.operations.swap(index, index - 1);
                            }
                            cx.notify();
                        })),
                )
                .child(
                    Button::new(("string-down", index))
                        .label("↓")
                        .ghost()
                        .disabled(index + 1 == draft.string.operations.len())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(editor) = &mut this.package_editor
                                && index + 1 < editor.policy.string.operations.len()
                            {
                                editor.policy.string.operations.swap(index, index + 1);
                            }
                            cx.notify();
                        })),
                )
                .when(operation.operator != StringSetOperator::Complement, |row| {
                    row.child(
                        Button::new(("string-edit", index))
                            .label(tr!("Edit").into_owned())
                            .ghost()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if let Some(editor) = &mut this.package_editor {
                                    editor.policy.string_condition = edit_operation.clone();
                                    editor.policy.string_edit_index = Some(index);
                                }
                                edit_input.update(cx, |state, cx| {
                                    state.set_value(edit_value.clone(), window, cx)
                                });
                                cx.notify();
                            })),
                    )
                })
                .child(
                    Button::new(("string-remove", index))
                        .icon(OrbitIcon::Trash)
                        .ghost()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            if let Some(editor) = &mut this.package_editor
                                && index < editor.policy.string.operations.len()
                            {
                                editor.policy.string.operations.remove(index);
                                editor.policy.string_edit_index = None;
                            }
                            cx.notify();
                        })),
                ),
        );
    }

    let condition = draft.string_condition.clone();
    let condition_value_missing =
        condition.needs_value() && input.read(cx).value().trim().is_empty();
    let set_operators = [
        (StringSetOperator::Intersect, tr!("Intersect").into_owned()),
        (StringSetOperator::Union, tr!("Union").into_owned()),
    ];
    let predicates = [
        (StringPredicate::Empty, tr!("Empty").into_owned()),
        (StringPredicate::Present, tr!("Present").into_owned()),
        (StringPredicate::Equals, tr!("Equals").into_owned()),
        (StringPredicate::Contains, tr!("Contains").into_owned()),
        (StringPredicate::StartsWith, tr!("Starts with").into_owned()),
        (StringPredicate::EndsWith, tr!("Ends with").into_owned()),
    ];
    let mut tokens = versions
        .into_iter()
        .flat_map(|versions| versions.candidates.iter())
        .flat_map(|candidate| candidate.string_tokens.iter().cloned())
        .collect::<Vec<_>>();
    tokens.sort();
    tokens.dedup();
    tokens.truncate(12);
    let editing = draft.string_edit_index;

    v_flex()
        .gap_3()
        .child(ui::section_title(
            tr!("Version string operations").into_owned(),
            tr!("Build a readable ordered filter over the complete JAR-declared version")
                .into_owned(),
            cx,
        ))
        .child(
            ui::compact_card(cx)
                .child(
                    div()
                        .text_sm()
                        .font_semibold()
                        .child(tr!("Initial candidate set").into_owned()),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(
                            Button::new("string-initial-all")
                                .label(tr!("Start with all").into_owned())
                                .selected(draft.string.initial_all)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(editor) = &mut this.package_editor {
                                        editor.policy.string.initial_all = true;
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("string-initial-none")
                                .label(tr!("Start with none").into_owned())
                                .selected(!draft.string.initial_all)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(editor) = &mut this.package_editor {
                                        editor.policy.string.initial_all = false;
                                    }
                                    cx.notify();
                                })),
                        ),
                ),
        )
        .child(
            h_flex()
                .justify_between()
                .items_center()
                .child(ui::section_title(
                    tr!("Ordered operations").into_owned(),
                    tr!("Each row transforms the result produced by the rows above it")
                        .into_owned(),
                    cx,
                ))
                .child(
                    Button::new("string-add-complement")
                        .label(tr!("Invert result").into_owned())
                        .on_click(cx.listener(|this, _, _, cx| {
                            if let Some(editor) = &mut this.package_editor {
                                editor.policy.string.operations.push(StringOperationDraft {
                                    operator: StringSetOperator::Complement,
                                    ..StringOperationDraft::default()
                                });
                            }
                            cx.notify();
                        })),
                ),
        )
        .when(draft.string.operations.is_empty(), |content| {
            content.child(
                div()
                    .px_3()
                    .py_2()
                    .rounded_lg()
                    .bg(cx.theme().secondary)
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr!("No operations; the initial set is used unchanged").into_owned()),
            )
        })
        .child(operations)
        .child(ui::section_title(
            if editing.is_some() {
                tr!("Edit operation").into_owned()
            } else {
                tr!("Add operation").into_owned()
            },
            tr!("Choose a set operation, a text test, and optional case handling").into_owned(),
            cx,
        ))
        .child(
            ui::compact_card(cx)
                .child(
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(tr!("Set operation").into_owned()),
                        )
                        .child(h_flex().gap_2().flex_wrap().children(
                            set_operators.into_iter().enumerate().map(
                                |(index, (operator, label))| {
                                    Button::new(("string-set-operator", index))
                                        .label(label)
                                        .selected(condition.operator == operator)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            if let Some(editor) = &mut this.package_editor {
                                                editor.policy.string_condition.operator = operator;
                                            }
                                            cx.notify();
                                        }))
                                },
                            ),
                        )),
                )
                .child(
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(tr!("Text test").into_owned()),
                        )
                        .child(
                            h_flex()
                                .gap_2()
                                .flex_wrap()
                                .child(
                                    Button::new("string-negated")
                                        .label(tr!("Negate condition").into_owned())
                                        .selected(condition.negated)
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            if let Some(editor) = &mut this.package_editor {
                                                editor.policy.string_condition.negated =
                                                    !editor.policy.string_condition.negated;
                                            }
                                            cx.notify();
                                        })),
                                )
                                .children(predicates.into_iter().enumerate().map(
                                    |(index, (predicate, label))| {
                                        Button::new(("string-predicate", index))
                                            .label(label)
                                            .selected(condition.predicate == predicate)
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                if let Some(editor) = &mut this.package_editor {
                                                    editor.policy.string_condition.predicate =
                                                        predicate;
                                                }
                                                cx.notify();
                                            }))
                                    },
                                )),
                        ),
                )
                .when(condition.needs_value(), |content| {
                    let token_input = input.clone();
                    content
                        .child(
                            h_flex().gap_2().child(Input::new(&input).flex_1()).child(
                                Button::new("string-case-sensitive")
                                    .label(if condition.case_sensitive {
                                        tr!("Case-sensitive").into_owned()
                                    } else {
                                        tr!("Ignore case").into_owned()
                                    })
                                    .selected(condition.case_sensitive)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(editor) = &mut this.package_editor {
                                            editor.policy.string_condition.case_sensitive =
                                                !editor.policy.string_condition.case_sensitive;
                                        }
                                        cx.notify();
                                    })),
                            ),
                        )
                        .when(!tokens.is_empty(), |content| {
                            content.child(h_flex().gap_1().flex_wrap().children(
                                tokens.into_iter().enumerate().map(|(index, token)| {
                                    let value = token.clone();
                                    let input = token_input.clone();
                                    Button::new(("string-token", index)).label(token).on_click(
                                        cx.listener(move |_, _, window, cx| {
                                            input.update(cx, |state, cx| {
                                                state.set_value(&value, window, cx)
                                            });
                                        }),
                                    )
                                }),
                            ))
                        })
                })
                .child(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .when(editing.is_some(), |row| {
                            row.child(
                                Button::new("string-edit-cancel")
                                    .label(tr!("Cancel edit").into_owned())
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        if let Some(editor) = &mut this.package_editor {
                                            editor.policy.string_edit_index = None;
                                            editor.policy.string_condition =
                                                StringOperationDraft::default();
                                        }
                                        this.inputs.string_value.update(cx, |state, cx| {
                                            state.set_value("", window, cx)
                                        });
                                        cx.notify();
                                    })),
                            )
                        })
                        .child(
                            Button::new("string-operation-apply")
                                .label(if editing.is_some() {
                                    tr!("Update operation").into_owned()
                                } else {
                                    tr!("Add operation").into_owned()
                                })
                                .primary()
                                .disabled(condition_value_missing)
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    let value = input_read.read(cx).value().trim().to_string();
                                    if let Some(editor) = &mut this.package_editor {
                                        let mut operation = editor.policy.string_condition.clone();
                                        operation.value = operation
                                            .needs_value()
                                            .then_some(value)
                                            .filter(|value| !value.is_empty());
                                        if operation.expression().is_some() {
                                            if let Some(index) =
                                                editor.policy.string_edit_index.take()
                                            {
                                                if index < editor.policy.string.operations.len() {
                                                    editor.policy.string.operations[index] =
                                                        operation;
                                                }
                                            } else {
                                                editor.policy.string.operations.push(operation);
                                            }
                                            editor.policy.string_condition =
                                                StringOperationDraft::default();
                                            input_read.update(cx, |state, cx| {
                                                state.set_value("", window, cx)
                                            });
                                        }
                                    }
                                    cx.notify();
                                })),
                        ),
                ),
        )
        .into_any_element()
}

fn render_toast(app: &OrbitApp, cx: &mut Context<OrbitApp>) -> impl IntoElement {
    let toast = app.toast.as_ref().expect("checked").clone();
    let color = match toast.kind {
        ToastKind::Warning => cx.theme().warning,
        ToastKind::Danger => cx.theme().danger,
    };
    h_flex()
        .relative()
        .absolute()
        .top(px(74.))
        .right(px(20.))
        .w(px(430.))
        .p_3()
        .gap_3()
        .rounded_lg()
        .border_1()
        .border_color(color)
        .shadow_lg()
        .bg(cx.theme().popover)
        .child(Icon::new(OrbitIcon::Warning).text_color(color))
        .child(div().flex_1().text_sm().child(toast.message))
        .child(
            Button::new("toast-close")
                .icon(OrbitIcon::Close)
                .ghost()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.toast = None;
                    cx.notify();
                })),
        )
        .with_animation(
            "toast-enter",
            Animation::new(Duration::from_millis(180)).with_easing(cubic_bezier(0.2, 0.8, 0.2, 1.)),
            |toast, delta| {
                toast
                    .right(px(-12.) + delta * px(32.))
                    .opacity(0.6 + delta * 0.4)
            },
        )
}

fn progress_percent(completed: Option<u64>, total: Option<u64>) -> Option<f32> {
    let (completed, total) = (completed?, total?);
    if total == 0 {
        return None;
    }
    Some((completed.min(total) as f64 * 100. / total as f64) as f32)
}

fn single_line_summary(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn task_state_color(state: TaskState, cx: &gpui::App) -> gpui::Hsla {
    match state {
        TaskState::Running => cx.theme().primary,
        TaskState::Succeeded => cx.theme().success,
        TaskState::Failed => cx.theme().danger,
        TaskState::Cancelled => cx.theme().muted_foreground,
    }
}

fn title_environment(value: &str) -> String {
    match value {
        "auto" => tr!("Automatic").into_owned(),
        "client" => tr!("Client").into_owned(),
        "server" => tr!("Server").into_owned(),
        "both" => tr!("Both").into_owned(),
        _ => value.to_string(),
    }
}

#[cfg(test)]
mod interaction_tests {
    use super::*;

    #[test]
    fn resolution_choice_contains_only_typed_package_actions() {
        let data = serde_json::json!({
            "changes": [{
                "different": true,
                "change": {
                    "package": "sodium",
                    "kind": "upgrade",
                    "current_version": "0.6.0",
                    "selected_version": "0.7.0",
                    "selected_description": "Modrinth · 3 dependency constraints",
                    "selected_artifact": "sodium-fabric-0.7.0.jar"
                }
            }]
        });

        let actions = interaction_package_actions(InteractionKind::Resolution, &data).unwrap();
        assert_eq!(actions.len(), 1);
        assert!(actions[0].different);
        assert_eq!(actions[0].change.package, "sodium");
        assert_eq!(actions[0].change.kind, "upgrade");
        assert_eq!(
            actions[0].change.selected_artifact.as_deref(),
            Some("sodium-fabric-0.7.0.jar")
        );

        let leaked_report_fields = serde_json::json!({
            "changes": [],
            "warnings": [],
            "diagnostics": []
        });
        assert!(
            interaction_package_actions(InteractionKind::Resolution, &leaked_report_fields)
                .is_err()
        );
    }

    #[test]
    fn confirmation_uses_actions_without_rendering_report_counters() {
        let data = serde_json::json!({
            "summary": { "installed": 1 },
            "changes": [{
                "package": "sodium",
                "kind": "install",
                "current_version": null,
                "selected_version": "0.7.0",
                "selected_description": null
            }],
            "warnings": [],
            "diagnostics": []
        });

        let actions = interaction_package_actions(InteractionKind::Confirmation, &data).unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].change.package, "sodium");
    }

    #[test]
    fn collapsed_activity_status_is_always_one_line() {
        assert_eq!(
            single_line_summary("Operation failed:\n dependency conflict\r\n user cancelled"),
            "Operation failed: dependency conflict user cancelled"
        );
    }
}
