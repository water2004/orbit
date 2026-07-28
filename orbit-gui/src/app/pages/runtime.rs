use gpui::{Context, IntoElement, ParentElement, Styled, Window, div, prelude::FluentBuilder as _};
use gpui_component::{
    ActiveTheme, Selectable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    v_flex,
};

use super::super::{
    Confirmation, ConfirmationAction, OrbitApp, RuntimeFlow, RuntimeFlowMode, RuntimeFlowStep,
};
use crate::app::components as ui;
use crate::app::controller::{loaders, title_case};
use crate::assets::OrbitIcon;
use crate::model::{LoaderVersion, MinecraftVersion};

pub(super) fn render(
    app: &mut OrbitApp,
    window: &mut Window,
    cx: &mut Context<OrbitApp>,
) -> gpui::AnyElement {
    if let Some(flow) = app.runtime_flow {
        render_flow(app, window, cx, flow).into_any_element()
    } else {
        render_dashboard(app, window, cx).into_any_element()
    }
}

fn render_dashboard(
    app: &mut OrbitApp,
    _window: &mut Window,
    cx: &mut Context<OrbitApp>,
) -> impl IntoElement {
    let selected = app.selected_instance().cloned();
    let actions = Button::new("runtime-create")
        .icon(OrbitIcon::Plus)
        .label(tr!("Create installation").into_owned())
        .primary()
        .on_click(cx.listener(|this, _, _, cx| {
            this.begin_runtime_flow(RuntimeFlowMode::Create);
            cx.notify();
        }));

    let mut content = v_flex().gap_4();
    if let Some(instance) = selected {
        let detail = app.instance_detail.clone();
        let installed = detail.as_ref().and_then(|item| item.installed.as_ref());
        let desired = detail.as_ref().map(|item| &item.desired);
        let current = installed.map_or_else(
            || tr!("Not installed").into_owned(),
            |item| {
                format!(
                    "{} · {} {}",
                    item.minecraft,
                    title_case(&item.loader),
                    item.loader_version.clone().unwrap_or_default()
                )
            },
        );
        let target = desired.map_or_else(
            || "—".to_string(),
            |item| {
                format!(
                    "{} · {} {}",
                    item.minecraft,
                    title_case(&item.loader),
                    item.loader_version.clone().unwrap_or_default()
                )
            },
        );
        let instance_id = instance.id.clone();
        content = content.child(
            ui::themed_card(cx)
                .child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .gap_3()
                        .child(
                            h_flex()
                                .gap_3()
                                .items_center()
                                .child(ui::icon_tile(if instance.kind == "server" { OrbitIcon::Server } else { OrbitIcon::Runtime }, cx))
                                .child(
                                    v_flex()
                                        .gap_1()
                                        .child(div().text_xl().font_semibold().child(instance.name.clone()))
                                        .child(div().text_xs().text_color(cx.theme().muted_foreground).child(instance.directory.display().to_string())),
                                ),
                        )
                        .child(
                            Button::new("runtime-launch")
                                .icon(OrbitIcon::Play)
                                .label(if instance.kind == "server" { tr!("Start server") } else { tr!("Launch game") }.into_owned())
                                .primary()
                                .on_click(cx.listener(|this, _, _, cx| { this.launch_selected(); cx.notify(); })),
                        ),
                )
                .child(ui::divider(cx))
                .child(ui::key_value(tr!("Installed").into_owned(), current, cx))
                .child(ui::key_value(tr!("Desired").into_owned(), target, cx))
                .children(instance.minecraft_directory.as_ref().map(|repository| {
                    ui::key_value(
                        tr!("Shared repository").into_owned(),
                        repository.display().to_string(),
                        cx,
                    )
                }))
                .children(detail.as_ref().map(|detail| {
                    ui::key_value(
                        tr!("Context").into_owned(),
                        detail.context.clone(),
                        cx,
                    )
                }))
                .child(ui::key_value(
                    tr!("Java").into_owned(),
                    installed.and_then(|item| item.java.as_ref()).map_or_else(
                        || tr!("Pending").into_owned(),
                        |java| format!(
                            "Java {} · {} · {} · {}",
                            java.major,
                            java.version,
                            java.provider,
                            java.platform
                        ),
                    ),
                    cx,
                ))
                .child(
                    h_flex()
                        .gap_2()
                        .flex_wrap()
                        .child(
                            Button::new("runtime-update")
                                .label(tr!("Change version").into_owned())
                                .on_click(cx.listener(|this, _, _, cx| { this.begin_runtime_flow(RuntimeFlowMode::Update); cx.notify(); })),
                        )
                        .child(
                            Button::new("runtime-repair")
                                .label(tr!("Verify and repair").into_owned())
                                .on_click(cx.listener(|this, _, _, cx| { this.install_runtime(); cx.notify(); })),
                        )
                        .when(!instance.is_default, |row| row.child(
                            Button::new("runtime-default")
                                .label(tr!("Make default").into_owned())
                                .ghost()
                                .on_click(cx.listener(|this, _, _, cx| { this.set_default_runtime(); cx.notify(); })),
                        ))
                        .child(
                            Button::new("runtime-unregister")
                                .label(tr!("Unregister").into_owned())
                                .ghost()
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.confirmation = Some(Confirmation {
                                        title: tr!("Unregister installation?").into_owned(),
                                        body: tr!("This removes the installation from Launcher without deleting its game directory.").into_owned(),
                                        action: ConfirmationAction::UnregisterInstance(instance_id.clone()),
                                    });
                                    cx.notify();
                                })),
                        ),
                ),
        );
    } else {
        let create = Button::new("runtime-empty-create")
            .icon(OrbitIcon::Plus)
            .label(tr!("Create installation").into_owned())
            .primary()
            .on_click(cx.listener(|this, _, _, cx| {
                this.begin_runtime_flow(RuntimeFlowMode::Create);
                cx.notify();
            }))
            .into_any_element();
        content = content.child(ui::themed_card(cx).child(ui::empty_state(
            OrbitIcon::Runtime,
            tr!("No managed installations").into_owned(),
            tr!("Create a client or server installation from the official Minecraft and Loader catalogs.").into_owned(),
            Some(create),
            cx,
        )));
    }

    let import_input = app.inputs.import_root.clone();
    let import_read = import_input.clone();
    let import_browse = import_input.clone();
    content = content
        .child(ui::section_title(
            tr!("Import existing installation").into_owned(),
            tr!("Launcher detection validates the selected directory").into_owned(),
            cx,
        ))
        .child(
            ui::compact_card(cx).child(
                h_flex()
                    .gap_2()
                    .child(
                        Input::new(&import_input)
                            .flex_1()
                            .prefix(gpui_component::Icon::new(OrbitIcon::Folder)),
                    )
                    .child(
                        Button::new("runtime-import-browse")
                            .label(tr!("Browse").into_owned())
                            .on_click(cx.listener(move |_, _, window, cx| {
                                OrbitApp::choose_directory(&import_browse, window, cx)
                            })),
                    )
                    .child(
                        Button::new("runtime-import-apply")
                            .label(tr!("Import").into_owned())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let root = import_read.read(cx).value().trim().to_string();
                                if !root.is_empty() {
                                    this.import_runtime(root);
                                }
                                cx.notify();
                            })),
                    ),
            ),
        );

    ui::page(
        tr!("Installations").into_owned(),
        tr!("Minecraft, Loader and Java lifecycle from official metadata").into_owned(),
        actions,
        content,
        cx,
    )
}

fn render_flow(
    app: &mut OrbitApp,
    window: &mut Window,
    cx: &mut Context<OrbitApp>,
    flow: RuntimeFlow,
) -> impl IntoElement {
    let actions = Button::new("runtime-flow-close")
        .label(tr!("Close").into_owned())
        .ghost()
        .on_click(cx.listener(|this, _, _, cx| {
            this.runtime_flow = None;
            cx.notify();
        }));
    let content = v_flex()
        .gap_4()
        .child(render_steps(flow.step, cx))
        .child(match flow.step {
            RuntimeFlowStep::Minecraft => {
                render_minecraft_step(app, window, cx, flow).into_any_element()
            }
            RuntimeFlowStep::Components => render_components_step(app, cx, flow).into_any_element(),
            RuntimeFlowStep::Review => render_review_step(app, window, cx, flow).into_any_element(),
        });
    ui::page(
        if flow.mode == RuntimeFlowMode::Create {
            tr!("Create installation")
        } else {
            tr!("Update installation")
        }
        .into_owned(),
        tr!("Choose exact catalog entries; installation uses one transactional path").into_owned(),
        actions,
        content,
        cx,
    )
}

fn render_steps(active: RuntimeFlowStep, cx: &gpui::App) -> impl IntoElement {
    let steps = [
        (RuntimeFlowStep::Minecraft, tr!("Minecraft").into_owned()),
        (RuntimeFlowStep::Components, tr!("Components").into_owned()),
        (RuntimeFlowStep::Review, tr!("Review").into_owned()),
    ];
    h_flex()
        .gap_2()
        .children(steps.into_iter().enumerate().map(|(index, (step, label))| {
            h_flex()
                .gap_2()
                .items_center()
                .child(ui::pill(
                    (index + 1).to_string(),
                    if step == active {
                        cx.theme().primary
                    } else {
                        cx.theme().secondary
                    },
                    if step == active {
                        cx.theme().primary_foreground
                    } else {
                        cx.theme().secondary_foreground
                    },
                ))
                .child(
                    div()
                        .text_sm()
                        .font_medium()
                        .text_color(if step == active {
                            cx.theme().foreground
                        } else {
                            cx.theme().muted_foreground
                        })
                        .child(label),
                )
        }))
}

fn render_minecraft_step(
    app: &mut OrbitApp,
    _window: &mut Window,
    cx: &mut Context<OrbitApp>,
    flow: RuntimeFlow,
) -> impl IntoElement {
    let filter = app
        .input_value(&app.inputs.minecraft_filter, cx)
        .to_ascii_lowercase();
    let active = if flow.mode == RuntimeFlowMode::Create {
        &app.new_instance.minecraft
    } else {
        &app.runtime_edit.minecraft
    };
    let mut list = v_flex().gap_3();
    if flow.mode == RuntimeFlowMode::Create {
        list = list
            .child(ui::section_title(
                tr!("Installation type").into_owned(),
                tr!(
                    "Client instances use the managed repository; servers use an explicit directory"
                )
                .into_owned(),
                cx,
            ))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("new-kind-client")
                            .icon(OrbitIcon::Runtime)
                            .label(tr!("Client").into_owned())
                            .selected(app.new_instance.kind == 0)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.new_instance.kind = 0;
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("new-kind-server")
                            .icon(OrbitIcon::Server)
                            .label(tr!("Server").into_owned())
                            .selected(app.new_instance.kind == 1)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.new_instance.kind = 1;
                                cx.notify();
                            })),
                    ),
            )
            .child(ui::section_title(
                tr!("Minecraft version").into_owned(),
                tr!("Select an exact entry from Mojang's official catalog").into_owned(),
                cx,
            ));
    }
    let list = list
        .child(
            h_flex()
                .gap_2()
                .child(ui::search_input(&app.inputs.minecraft_filter).flex_1())
                .children(
                    [
                        tr!("Release"),
                        tr!("Snapshot"),
                        tr!("Historical"),
                        tr!("All"),
                    ]
                    .into_iter()
                    .enumerate()
                    .map(|(index, label)| {
                        Button::new(("minecraft-kind", index))
                            .label(label.into_owned())
                            .ghost()
                            .selected(app.minecraft_version_type == index)
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.minecraft_version_type = index;
                                cx.notify();
                            }))
                    }),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(
                    tr!("Showing at most 120 matches; filter to reach older releases.")
                        .into_owned(),
                ),
        );
    let matches: Vec<MinecraftVersion> = app
        .minecraft_versions
        .iter()
        .filter(|version| {
            let type_matches =
                minecraft_version_matches_filter(&version.version_type, app.minecraft_version_type);
            type_matches && (filter.is_empty() || version.id.to_ascii_lowercase().contains(&filter))
        })
        .take(120)
        .cloned()
        .collect();
    let mut rows = ui::themed_card(cx).p_0().gap_0();
    for (index, version) in matches.into_iter().enumerate() {
        if index > 0 {
            rows = rows.child(ui::divider(cx));
        }
        let version_id = version.id.clone();
        let name_input = app.inputs.new_name.clone();
        rows = rows.child(
            Button::new(("minecraft-version", index))
                .ghost()
                .selected(active == &version.id)
                .w_full()
                .rounded(gpui_component::button::ButtonRounded::None)
                .child(
                    h_flex()
                        .w_full()
                        .px_2()
                        .py_1()
                        .justify_between()
                        .child(
                            h_flex()
                                .gap_2()
                                .child(div().font_semibold().child(version.id))
                                .child(ui::neutral_pill(title_case(&version.version_type), cx))
                                .when(version.latest_release || version.latest_snapshot, |row| {
                                    row.child(ui::pill(
                                        tr!("Latest").into_owned(),
                                        cx.theme().primary.opacity(0.13),
                                        cx.theme().primary,
                                    ))
                                }),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(version.release_time),
                        ),
                )
                .on_click(cx.listener(move |this, _, window, cx| {
                    if flow.mode == RuntimeFlowMode::Create {
                        this.new_instance.minecraft = version_id.clone();
                        this.new_instance.loader_version.clear();
                        let suggestion = suggested_instance_name(
                            &version_id,
                            loaders()[this.new_instance.loader],
                        );
                        this.new_instance.name = suggestion.clone();
                        name_input.update(cx, |input, cx| input.set_value(suggestion, window, cx));
                    } else {
                        this.runtime_edit.minecraft = version_id.clone();
                        this.runtime_edit.loader_version.clear();
                    }
                    let loader = if flow.mode == RuntimeFlowMode::Create {
                        this.new_instance.loader
                    } else {
                        this.runtime_edit.loader
                    };
                    this.request_runtime_metadata(&version_id, loader);
                    this.runtime_flow = Some(RuntimeFlow {
                        step: RuntimeFlowStep::Components,
                        ..flow
                    });
                    cx.notify();
                })),
        );
    }
    list.child(rows)
}

fn minecraft_version_matches_filter(version_type: &str, filter: usize) -> bool {
    match filter {
        0 => version_type == "release",
        1 => version_type == "snapshot",
        2 => matches!(version_type, "old_alpha" | "old_beta"),
        3 => true,
        _ => false,
    }
}

fn render_components_step(
    app: &mut OrbitApp,
    cx: &mut Context<OrbitApp>,
    flow: RuntimeFlow,
) -> impl IntoElement {
    let (minecraft, loader_index, selected_version) = if flow.mode == RuntimeFlowMode::Create {
        (
            app.new_instance.minecraft.clone(),
            app.new_instance.loader,
            app.new_instance.loader_version.clone(),
        )
    } else {
        (
            app.runtime_edit.minecraft.clone(),
            app.runtime_edit.loader,
            app.runtime_edit.loader_version.clone(),
        )
    };
    let mut body = v_flex()
        .gap_4()
        .child(ui::section_title(
            tr!("Loader").into_owned(),
            minecraft.clone(),
            cx,
        ))
        .child(
            h_flex()
                .gap_2()
                .flex_wrap()
                .children(loaders().into_iter().enumerate().map(|(index, loader)| {
                    let minecraft = minecraft.clone();
                    let name_input = app.inputs.new_name.clone();
                    Button::new(("loader", index))
                        .label(title_case(loader))
                        .selected(loader_index == index)
                        .on_click(cx.listener(move |this, _, window, cx| {
                            if flow.mode == RuntimeFlowMode::Create {
                                let current = name_input.read(cx).value().to_string();
                                let replace_name =
                                    current.trim().is_empty() || current == this.new_instance.name;
                                this.new_instance.loader = index;
                                this.new_instance.loader_version.clear();
                                if replace_name {
                                    let suggestion = suggested_instance_name(&minecraft, loader);
                                    this.new_instance.name = suggestion.clone();
                                    name_input.update(cx, |input, cx| {
                                        input.set_value(suggestion, window, cx)
                                    });
                                }
                            } else {
                                this.runtime_edit.loader = index;
                                this.runtime_edit.loader_version.clear();
                            }
                            this.request_runtime_metadata(&minecraft, index);
                            cx.notify();
                        }))
                })),
        );
    if loader_index == 0 {
        let java = app
            .java_requirements
            .get(&minecraft)
            .and_then(|item| item.major)
            .map_or_else(
                || tr!("Resolving…").into_owned(),
                |major| format!("Java {major}"),
            );
        return body
            .child(ui::compact_card(cx).child(ui::key_value(
                tr!("Required Java").into_owned(),
                java,
                cx,
            )))
            .child(
                h_flex().justify_end().child(
                    Button::new("components-next-vanilla")
                        .label(tr!("Review").into_owned())
                        .primary()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.runtime_flow = Some(RuntimeFlow {
                                step: RuntimeFlowStep::Review,
                                ..flow
                            });
                            cx.notify();
                        })),
                ),
            );
    }
    let key = (loaders()[loader_index].to_string(), minecraft.clone());
    let versions = app
        .loader_version_catalogs
        .get(&key)
        .cloned()
        .unwrap_or_default();
    body = body.child(ui::section_title(
        tr!("Compatible Loader versions").into_owned(),
        tr!("Provider metadata for the selected Minecraft version").into_owned(),
        cx,
    ));
    if versions.is_empty() {
        body = body.child(
            ui::themed_card(cx).child(ui::empty_state(
                OrbitIcon::Refresh,
                tr!("Loading Loader versions").into_owned(),
                tr!("Compatible entries are being fetched from the Loader's official metadata.")
                    .into_owned(),
                None,
                cx,
            )),
        );
    } else {
        let mut rows = ui::themed_card(cx).p_0().gap_0();
        for (index, version) in versions.into_iter().take(100).enumerate() {
            if index > 0 {
                rows = rows.child(ui::divider(cx));
            }
            rows = rows.child(loader_version_row(
                version,
                index,
                selected_version.clone(),
                minecraft.clone(),
                flow,
                cx,
            ));
        }
        body = body.child(rows);
    }
    body
}

fn loader_version_row(
    version: LoaderVersion,
    index: usize,
    selected: String,
    _minecraft: String,
    flow: RuntimeFlow,
    cx: &mut Context<OrbitApp>,
) -> impl IntoElement {
    let version_id = version.version.clone();
    Button::new(("loader-version", index))
        .ghost()
        .selected(selected == version.version)
        .w_full()
        .rounded(gpui_component::button::ButtonRounded::None)
        .child(
            h_flex()
                .w_full()
                .px_2()
                .py_1()
                .gap_2()
                .child(div().font_semibold().child(version.version))
                .when(version.recommended, |row| {
                    row.child(ui::pill(
                        tr!("Recommended").into_owned(),
                        cx.theme().success.opacity(0.13),
                        cx.theme().success,
                    ))
                })
                .when(version.stable, |row| {
                    row.child(ui::neutral_pill(tr!("Stable").into_owned(), cx))
                })
                .when(version.latest, |row| {
                    row.child(ui::pill(
                        tr!("Latest").into_owned(),
                        cx.theme().primary.opacity(0.13),
                        cx.theme().primary,
                    ))
                })
                .when_some(version.minimum_java_major, |row, major| {
                    row.child(
                        div()
                            .ml_auto()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("Java {major}+")),
                    )
                }),
        )
        .on_click(cx.listener(move |this, _, _, cx| {
            if flow.mode == RuntimeFlowMode::Create {
                this.new_instance.loader_version = version_id.clone();
            } else {
                this.runtime_edit.loader_version = version_id.clone();
            }
            this.runtime_flow = Some(RuntimeFlow {
                step: RuntimeFlowStep::Review,
                ..flow
            });
            cx.notify();
        }))
}

fn render_review_step(
    app: &mut OrbitApp,
    _window: &mut Window,
    cx: &mut Context<OrbitApp>,
    flow: RuntimeFlow,
) -> impl IntoElement {
    let (minecraft, loader, loader_version) = if flow.mode == RuntimeFlowMode::Create {
        (
            app.new_instance.minecraft.clone(),
            app.new_instance.loader,
            app.new_instance.loader_version.clone(),
        )
    } else {
        (
            app.runtime_edit.minecraft.clone(),
            app.runtime_edit.loader,
            app.runtime_edit.loader_version.clone(),
        )
    };
    let java = app
        .java_requirements
        .get(&minecraft)
        .and_then(|item| item.major)
        .map_or_else(
            || tr!("Automatic").into_owned(),
            |major| format!("Java {major}"),
        );
    let mut body = v_flex().gap_4().child(
        ui::themed_card(cx)
            .child(ui::key_value(tr!("Minecraft").into_owned(), minecraft, cx))
            .child(ui::key_value(
                tr!("Loader").into_owned(),
                if loader == 0 {
                    title_case(loaders()[loader])
                } else {
                    format!("{} {}", title_case(loaders()[loader]), loader_version)
                },
                cx,
            ))
            .child(ui::key_value(tr!("Java").into_owned(), java, cx)),
    );
    if flow.mode == RuntimeFlowMode::Create {
        let name = app.inputs.new_name.clone();
        let server_directory = app.inputs.new_server_directory.clone();
        let directory_browse = server_directory.clone();
        body = body.child(ui::field(
            tr!("Installation name").into_owned(),
            tr!("Used by global Launcher instance selection").into_owned(),
            &name,
            cx,
        ));
        if app.new_instance.kind == 0 {
            body = body.child(
                ui::themed_card(cx).child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr!("This client will use the managed Minecraft directory and an isolated versions/<instance> game directory.").into_owned()),
                ),
            );
        } else {
            body = body.child(
                v_flex()
                    .gap_1p5()
                    .child(
                        div()
                            .text_sm()
                            .font_semibold()
                            .child(tr!("Server directory").into_owned()),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Input::new(&server_directory)
                                    .flex_1()
                                    .prefix(gpui_component::Icon::new(OrbitIcon::Folder)),
                            )
                            .child(
                                Button::new("new-server-directory-browse")
                                    .label(tr!("Browse").into_owned())
                                    .on_click(cx.listener(move |_, _, window, cx| {
                                        OrbitApp::choose_directory(&directory_browse, window, cx)
                                    })),
                            ),
                    ),
            );
        }
        body = body.child(
            h_flex()
                .justify_end()
                .gap_2()
                .child(
                    Button::new("review-back")
                        .label(tr!("Back").into_owned())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.runtime_flow = Some(RuntimeFlow {
                                step: RuntimeFlowStep::Components,
                                ..flow
                            });
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("review-create")
                        .icon(OrbitIcon::Download)
                        .label(tr!("Create and install").into_owned())
                        .primary()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.new_instance.name = name.read(cx).value().trim().to_string();
                            this.new_instance.server_directory =
                                server_directory.read(cx).value().trim().to_string();
                            if !this.new_instance.name.is_empty()
                                && (this.new_instance.kind == 0
                                    || !this.new_instance.server_directory.is_empty())
                            {
                                this.create_runtime();
                                this.runtime_flow = None;
                            }
                            cx.notify();
                        })),
                ),
        );
    } else {
        body = body.child(
            h_flex()
                .justify_end()
                .gap_2()
                .child(
                    Button::new("review-update-back")
                        .label(tr!("Back").into_owned())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.runtime_flow = Some(RuntimeFlow {
                                step: RuntimeFlowStep::Components,
                                ..flow
                            });
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("review-update")
                        .icon(OrbitIcon::Download)
                        .label(tr!("Apply and install").into_owned())
                        .primary()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.configure_runtime_and_install();
                            this.runtime_flow = None;
                            cx.notify();
                        })),
                ),
        );
    }
    body
}

fn suggested_instance_name(minecraft: &str, loader: &str) -> String {
    if loader == "vanilla" {
        minecraft.to_string()
    } else {
        format!("{}-{}", minecraft, title_case(loader))
    }
}

#[cfg(test)]
mod tests {
    use super::minecraft_version_matches_filter;

    #[test]
    fn minecraft_channels_are_not_conflated() {
        assert!(minecraft_version_matches_filter("release", 0));
        assert!(!minecraft_version_matches_filter("snapshot", 0));
        assert!(minecraft_version_matches_filter("snapshot", 1));
        assert!(!minecraft_version_matches_filter("old_beta", 1));
        assert!(minecraft_version_matches_filter("old_alpha", 2));
        assert!(minecraft_version_matches_filter("release", 3));
        assert!(!minecraft_version_matches_filter("release", 99));
    }
}
