use gpui::{Context, IntoElement, ParentElement, Styled, Window, div, px};
use gpui_component::{
    ActiveTheme, StyledExt,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};

use super::super::{OrbitApp, RuntimeFlowMode};
use crate::app::components as ui;
use crate::assets::OrbitIcon;
use crate::model::Page;

pub(super) fn render(
    app: &mut OrbitApp,
    _window: &mut Window,
    cx: &mut Context<OrbitApp>,
) -> impl IntoElement {
    let instance = app.selected_instance().cloned();
    let actions = if instance.is_some() {
        Button::new("home-launch")
            .icon(OrbitIcon::Play)
            .label(
                if app.is_server() {
                    tr!("Start server")
                } else {
                    tr!("Launch game")
                }
                .into_owned(),
            )
            .primary()
            .on_click(cx.listener(|this, _, _, cx| {
                this.launch_selected();
                cx.notify();
            }))
            .into_any_element()
    } else {
        div().into_any_element()
    };

    let content = if let Some(instance) = instance {
        let detail = app.instance_detail.clone();
        let installed = detail.as_ref().and_then(|item| item.installed.as_ref());
        let minecraft = installed
            .map(|item| item.minecraft.as_str())
            .unwrap_or_else(|| {
                detail
                    .as_ref()
                    .map_or("—", |item| item.desired.minecraft.as_str())
            });
        let loader = installed
            .map(|item| item.loader.as_str())
            .unwrap_or_else(|| {
                detail
                    .as_ref()
                    .map_or("—", |item| item.desired.loader.as_str())
            });
        let java = installed.and_then(|item| item.java.as_ref()).map_or_else(
            || tr!("Pending").into_owned(),
            |java| format!("Java {}", java.major),
        );
        let runtime_ready = installed.is_some();
        let orbit_ready = instance.directory.join("orbit.toml").is_file();

        v_flex()
            .gap_4()
            .child(
                ui::themed_card(cx)
                    .child(
                        h_flex()
                            .items_center()
                            .justify_between()
                            .gap_4()
                            .child(
                                h_flex()
                                    .min_w_0()
                                    .gap_3()
                                    .items_center()
                                    .child(ui::icon_tile(
                                        if instance.kind == "server" {
                                            OrbitIcon::Server
                                        } else {
                                            OrbitIcon::Home
                                        },
                                        cx,
                                    ))
                                    .child(
                                        v_flex()
                                            .min_w_0()
                                            .gap_1()
                                            .child(
                                                div()
                                                    .text_xl()
                                                    .font_semibold()
                                                    .child(instance.name.clone()),
                                            )
                                            .child(
                                                div()
                                                    .text_sm()
                                                    .text_color(cx.theme().muted_foreground)
                                                    .overflow_hidden()
                                                    .child(
                                                        instance.directory.display().to_string(),
                                                    ),
                                            ),
                                    ),
                            )
                            .child(ui::pill(
                                if runtime_ready {
                                    tr!("Ready")
                                } else {
                                    tr!("Not installed")
                                }
                                .into_owned(),
                                ui::state_color(runtime_ready, cx).opacity(0.14),
                                ui::state_color(runtime_ready, cx),
                            )),
                    )
                    .child(ui::divider(cx))
                    .child(
                        h_flex()
                            .gap_6()
                            .flex_wrap()
                            .child(ui::key_value(
                                tr!("Minecraft").into_owned(),
                                minecraft.to_string(),
                                cx,
                            ))
                            .child(ui::key_value(
                                tr!("Loader").into_owned(),
                                crate::app::controller::title_case(loader),
                                cx,
                            ))
                            .child(ui::key_value(tr!("Java").into_owned(), java, cx))
                            .child(ui::key_value(
                                tr!("Kind").into_owned(),
                                crate::app::controller::title_case(&instance.kind),
                                cx,
                            )),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .flex_wrap()
                            .child(
                                Button::new("home-change")
                                    .label(tr!("Change version").into_owned())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.begin_runtime_flow(RuntimeFlowMode::Update);
                                        this.preferences.page = Page::Runtime;
                                        cx.notify();
                                    })),
                            )
                            .child(
                                Button::new("home-repair")
                                    .label(tr!("Verify and repair").into_owned())
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        this.install_runtime();
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .child(
                h_flex()
                    .gap_3()
                    .flex_wrap()
                    .child(ui::metric(
                        tr!("Mods").into_owned(),
                        app.packages.len().to_string(),
                        if orbit_ready {
                            tr!("Managed logical packages")
                        } else {
                            tr!("Orbit is not initialized")
                        }
                        .into_owned(),
                        cx,
                    ))
                    .child(ui::metric(
                        tr!("Updates").into_owned(),
                        if app.outdated_checked {
                            app.outdated.len().to_string()
                        } else {
                            "—".to_string()
                        },
                        tr!("Latest feasible solver result").into_owned(),
                        cx,
                    ))
                    .child(ui::metric(
                        tr!("Compatibility").into_owned(),
                        app.audit
                            .as_ref()
                            .map_or_else(|| "—".to_string(), |audit| audit.readiness.clone()),
                        tr!("Latest bytecode audit").into_owned(),
                        cx,
                    )),
            )
            .child(ui::section_title(
                tr!("Quick actions").into_owned(),
                tr!("Common workspace tasks").into_owned(),
                cx,
            ))
            .child(
                h_flex()
                    .gap_3()
                    .flex_wrap()
                    .child(quick_action(
                        "quick-mods",
                        OrbitIcon::Mods,
                        tr!("Manage mods").into_owned(),
                        tr!("Install, sync and update logical packages").into_owned(),
                        cx.listener(|this, _, _, cx| {
                            this.preferences.page = Page::Library;
                            cx.notify();
                        }),
                        cx,
                    ))
                    .child(quick_action(
                        "quick-browse",
                        OrbitIcon::Browse,
                        tr!("Browse projects").into_owned(),
                        tr!("Search compatible provider catalogs").into_owned(),
                        cx.listener(|this, _, _, cx| {
                            this.preferences.page = Page::Discover;
                            cx.notify();
                        }),
                        cx,
                    ))
                    .child(quick_action(
                        "quick-audit",
                        OrbitIcon::Audit,
                        tr!("Run compatibility audit").into_owned(),
                        tr!("Inspect bytecode and active Mixin risk").into_owned(),
                        cx.listener(|this, _, _, cx| {
                            this.preferences.page = Page::Audit;
                            this.run_audit();
                            cx.notify();
                        }),
                        cx,
                    )),
            )
            .into_any_element()
    } else {
        ui::themed_card(cx)
            .child(ui::empty_state(
                OrbitIcon::Runtime,
                tr!("No installation selected").into_owned(),
                tr!(
                    "Create a managed Minecraft installation or import an existing game directory."
                )
                .into_owned(),
                Some(
                    h_flex()
                        .pt_2()
                        .gap_2()
                        .child(
                            Button::new("empty-create")
                                .icon(OrbitIcon::Plus)
                                .label(tr!("Create installation").into_owned())
                                .primary()
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.begin_runtime_flow(RuntimeFlowMode::Create);
                                    this.preferences.page = Page::Runtime;
                                    cx.notify();
                                })),
                        )
                        .into_any_element(),
                ),
                cx,
            ))
            .into_any_element()
    };

    ui::page(
        tr!("Home").into_owned(),
        tr!("Your current Minecraft workspace at a glance").into_owned(),
        actions,
        content,
        cx,
    )
}

fn quick_action(
    id: &'static str,
    icon: OrbitIcon,
    title: String,
    detail: String,
    handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
    cx: &gpui::App,
) -> impl IntoElement {
    Button::new(id)
        .ghost()
        .w(px(250.))
        .h(px(78.))
        .p_3()
        .justify_start()
        .child(
            h_flex()
                .gap_3()
                .items_center()
                .child(ui::icon_tile(icon, cx))
                .child(
                    v_flex()
                        .gap_1()
                        .items_start()
                        .child(div().font_semibold().child(title))
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(detail),
                        ),
                ),
        )
        .on_click(handler)
}
