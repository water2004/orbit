use gpui::{Context, IntoElement, ParentElement, Styled, Window, div, prelude::FluentBuilder as _};
use gpui_component::{
    ActiveTheme, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    v_flex,
};

use super::super::OrbitApp;
use crate::app::components as ui;
use crate::assets::OrbitIcon;

pub(super) fn render(
    app: &mut OrbitApp,
    _window: &mut Window,
    cx: &mut Context<OrbitApp>,
) -> impl IntoElement {
    let Some(instance) = app.selected_instance().cloned() else {
        return ui::page(
            tr!("Server").into_owned(),
            tr!("Process supervision, console and Minecraft EULA").into_owned(),
            div(),
            ui::themed_card(cx).child(ui::empty_state(
                OrbitIcon::Server,
                tr!("No server selected").into_owned(),
                tr!("Select a server installation from the top bar.").into_owned(),
                None,
                cx,
            )),
            cx,
        );
    };
    let running = app
        .server_status
        .as_ref()
        .is_some_and(|status| status.running);
    let actions = Button::new("server-toggle")
        .icon(if running {
            OrbitIcon::Close
        } else {
            OrbitIcon::Play
        })
        .label(
            if running {
                tr!("Stop server")
            } else {
                tr!("Start server")
            }
            .into_owned(),
        )
        .with_variant(if running {
            gpui_component::button::ButtonVariant::Danger
        } else {
            gpui_component::button::ButtonVariant::Primary
        })
        .on_click(cx.listener(move |this, _, _, cx| {
            this.server_action(if running { "stop" } else { "start" });
            cx.notify();
        }));
    let command_input = app.inputs.server_command.clone();
    let command_read = command_input.clone();
    let status_detail = app
        .server_status
        .as_ref()
        .and_then(|status| status.state.as_ref());
    let content = v_flex()
        .gap_4()
        .child(
            ui::themed_card(cx)
                .child(
                    h_flex()
                        .gap_3()
                        .items_center()
                        .child(ui::icon_tile(OrbitIcon::Server, cx))
                        .child(
                            v_flex()
                                .flex_1()
                                .gap_1()
                                .child(div().text_xl().font_semibold().child(instance.name))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(instance.directory.display().to_string()),
                                ),
                        )
                        .child(ui::pill(
                            if running {
                                tr!("Running")
                            } else {
                                tr!("Stopped")
                            }
                            .into_owned(),
                            ui::state_color(running, cx).opacity(0.14),
                            ui::state_color(running, cx),
                        )),
                )
                .when_some(status_detail, |card, state| {
                    card.child(ui::render_json_summary(state, cx))
                }),
        )
        .child(
            h_flex()
                .gap_3()
                .items_start()
                .child(
                    ui::themed_card(cx)
                        .flex_1()
                        .child(ui::section_title(
                            tr!("Console command").into_owned(),
                            tr!("Sent to the supervised server process").into_owned(),
                            cx,
                        ))
                        .child(
                            h_flex()
                                .gap_2()
                                .child(
                                    Input::new(&command_input)
                                        .flex_1()
                                        .prefix(gpui_component::Icon::new(OrbitIcon::Terminal)),
                                )
                                .child(
                                    Button::new("server-send")
                                        .label(tr!("Send").into_owned())
                                        .primary()
                                        .on_click(cx.listener(move |this, _, window, cx| {
                                            let command =
                                                command_read.read(cx).value().trim().to_string();
                                            if !command.is_empty() {
                                                this.send_server_command(command);
                                                command_read.update(cx, |state, cx| {
                                                    state.set_value("", window, cx)
                                                });
                                            }
                                            cx.notify();
                                        })),
                                ),
                        ),
                )
                .child(
                    ui::themed_card(cx)
                        .flex_1()
                        .child(ui::section_title(
                            "Minecraft EULA",
                            tr!("Required before first server launch").into_owned(),
                            cx,
                        ))
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(
                                    tr!("View the complete current document before accepting.")
                                        .into_owned(),
                                ),
                        )
                        .child(
                            Button::new("server-eula")
                                .label(tr!("Show EULA").into_owned())
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.server_action("eula");
                                    cx.notify();
                                })),
                        ),
                ),
        );
    ui::page(
        tr!("Server").into_owned(),
        tr!("Process supervision, console and Minecraft EULA").into_owned(),
        actions,
        content,
        cx,
    )
}
