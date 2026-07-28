use std::time::Duration;

use gpui::{
    Animation, AnimationExt, AnyElement, Context, IntoElement, ParentElement, Styled, Window, div,
    ease_in_out, prelude::FluentBuilder as _, px, relative,
};
use gpui_component::{
    ActiveTheme, Disableable, Icon, Selectable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    progress::Progress,
    scroll::ScrollableElement,
    v_flex,
};

use super::super::{OrbitApp, TaskState, ToastKind};
use crate::app::components as ui;
use crate::assets::OrbitIcon;

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
    v_flex()
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
                .child(div().font_medium().child(task.label.clone()))
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .overflow_hidden()
                        .flex_1()
                        .child(task.status_line.clone()),
                )
                .when(completed.is_some() || total.is_some(), |row| {
                    row.child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(match (completed, total) {
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
                            this.activity_open = !this.activity_open;
                            cx.notify();
                        })),
                ),
        )
        .child(match progress {
            Some(value) => Progress::new().h(px(6.)).value(value).into_any_element(),
            None if running => indeterminate(cx).into_any_element(),
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

fn indeterminate(cx: &gpui::App) -> impl IntoElement {
    div()
        .relative()
        .h(px(6.))
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
    if app.activity_open {
        overlays.push(render_drawer(app, cx).into_any_element());
    }

    if app.interaction.is_some() {
        overlays.push(render_interaction(app, cx).into_any_element());
    } else if app.confirmation.is_some() {
        overlays.push(render_confirmation(app, cx).into_any_element());
    } else if app.microsoft_session.is_some() {
        overlays.push(render_microsoft(app, cx).into_any_element());
    } else if app.eula_document.is_some() {
        overlays.push(render_eula(app, cx).into_any_element());
    } else if app.package_editor.is_some() {
        overlays.push(render_package_editor(app, cx).into_any_element());
    }
    if app.toast.is_some() {
        overlays.push(render_toast(app, cx).into_any_element());
    }
    overlays
}

fn render_drawer(app: &OrbitApp, cx: &mut Context<OrbitApp>) -> impl IntoElement {
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
                            this.activity_open = false;
                            cx.notify();
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
}

fn render_interaction(app: &OrbitApp, cx: &mut Context<OrbitApp>) -> impl IntoElement {
    let pending = app.interaction.as_ref().expect("checked").clone();
    let mut choices = v_flex().gap_2();
    for (index, choice) in pending.envelope.choices.iter().cloned().enumerate() {
        let choice_id = choice.id.clone();
        let different = has_difference(&choice.data);
        choices = choices.child(
            Button::new(("interaction-choice", index))
                .ghost()
                .w_full()
                .p_3()
                .selected(different)
                .child(
                    v_flex()
                        .w_full()
                        .gap_2()
                        .items_start()
                        .child(
                            h_flex()
                                .w_full()
                                .gap_2()
                                .child(if different { "◆" } else { "◇" })
                                .child(div().font_semibold().child(choice.label))
                                .when_some(choice.description, |row, description| {
                                    row.child(
                                        div()
                                            .ml_auto()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(description),
                                    )
                                }),
                        )
                        .child(ui::render_json_summary(&choice.data, cx)),
                )
                .on_click(cx.listener(move |this, _, _, cx| {
                    this.answer_interaction(Some(choice_id.clone()));
                    cx.notify();
                })),
        );
    }
    ui::modal_backdrop(
        ui::modal(
            700.,
            v_flex()
                .gap_4()
                .child(
                    div()
                        .text_xl()
                        .font_semibold()
                        .child(pending.envelope.prompt),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr!("◆ marks actions that differ between choices.").into_owned()),
                )
                .child(div().max_h(px(470.)).overflow_y_scrollbar().child(choices))
                .when(pending.envelope.allow_cancel, |modal| {
                    modal.child(
                        h_flex().justify_end().child(
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
    ui::modal_backdrop(
        ui::modal(
            620.,
            v_flex()
                .gap_4()
                .child(
                    h_flex()
                        .justify_between()
                        .child(div().text_xl().font_semibold().child(package_id.clone()))
                        .child(Button::new("package-close").icon(OrbitIcon::Close).ghost().on_click(cx.listener(|this, _, _, cx| { this.package_editor = None; cx.notify(); }))),
                )
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
                                this.add_package_remote(&package_id, provider, &locator);
                                remote_read.update(cx, |state, cx| state.set_value("", window, cx));
                                this.package_editor = None;
                            }
                            cx.notify();
                        }))),
                ),
            cx,
        ),
        cx,
    )
}

fn render_toast(app: &OrbitApp, cx: &mut Context<OrbitApp>) -> impl IntoElement {
    let toast = app.toast.as_ref().expect("checked").clone();
    let color = match toast.kind {
        ToastKind::Warning => cx.theme().warning,
        ToastKind::Danger => cx.theme().danger,
    };
    h_flex()
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
}

fn progress_percent(completed: Option<u64>, total: Option<u64>) -> Option<f32> {
    let (completed, total) = (completed?, total?);
    if total == 0 {
        return None;
    }
    Some((completed.min(total) as f64 * 100. / total as f64) as f32)
}

fn task_state_color(state: TaskState, cx: &gpui::App) -> gpui::Hsla {
    match state {
        TaskState::Running => cx.theme().primary,
        TaskState::Succeeded => cx.theme().success,
        TaskState::Failed => cx.theme().danger,
        TaskState::Cancelled => cx.theme().muted_foreground,
    }
}

fn has_difference(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Object(object) => {
            object.get("different").and_then(serde_json::Value::as_bool) == Some(true)
                || object.values().any(has_difference)
        }
        serde_json::Value::Array(values) => values.iter().any(has_difference),
        _ => false,
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
