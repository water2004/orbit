use gpui::{Context, IntoElement, ParentElement, Styled, Window, div, prelude::FluentBuilder as _};
use gpui_component::{
    ActiveTheme, Selectable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};

use super::super::{Confirmation, ConfirmationAction, OrbitApp, PackageEditor};
use crate::app::components as ui;
use crate::assets::OrbitIcon;

pub(super) fn render(
    app: &mut OrbitApp,
    _window: &mut Window,
    cx: &mut Context<OrbitApp>,
) -> impl IntoElement {
    let selected = app.selected_instance().cloned();
    let initialized = selected
        .as_ref()
        .is_some_and(|instance| instance.directory.join("orbit.toml").is_file());
    let filter = app
        .input_value(&app.inputs.package_filter, cx)
        .to_ascii_lowercase();
    let actions = h_flex()
        .gap_2()
        .child(
            Button::new("mods-refresh")
                .icon(OrbitIcon::Refresh)
                .tooltip(tr!("Refresh").into_owned())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.reload_packages();
                    cx.notify();
                })),
        )
        .child(
            Button::new("mods-sync")
                .label(tr!("Sync").into_owned())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.sync_instance();
                    cx.notify();
                })),
        )
        .child(
            Button::new("mods-fix")
                .label(tr!("Fix").into_owned())
                .primary()
                .on_click(cx.listener(|this, _, _, cx| {
                    this.fix_mods();
                    cx.notify();
                })),
        )
        .child(
            Button::new("mods-install")
                .icon(OrbitIcon::Download)
                .label(tr!("Install").into_owned())
                .on_click(cx.listener(|this, _, _, cx| {
                    this.install_mods();
                    cx.notify();
                })),
        );

    let content = if selected.is_none() {
        ui::themed_card(cx)
            .child(ui::empty_state(
                OrbitIcon::Runtime,
                tr!("No installation selected").into_owned(),
                tr!("Select a managed installation before editing its mod environment.")
                    .into_owned(),
                None,
                cx,
            ))
            .into_any_element()
    } else if !initialized {
        ui::themed_card(cx)
            .child(ui::empty_state(
                OrbitIcon::Mods,
                tr!("Orbit is not initialized").into_owned(),
                tr!("Create the workspace from the exact Minecraft and Loader versions already locked by Launcher.").into_owned(),
                Some(
                    Button::new("mods-init")
                        .label(tr!("Initialize Orbit").into_owned())
                        .primary()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.initialize_orbit();
                            cx.notify();
                        }))
                        .into_any_element(),
                ),
                cx,
            ))
            .into_any_element()
    } else {
        let mut body = v_flex().gap_3().child(
            h_flex()
                .gap_2()
                .items_center()
                .child(ui::search_input(&app.inputs.package_filter).flex_1())
                .child(
                    Button::new("mods-tab-installed")
                        .label(format!("{} {}", tr!("Installed"), app.packages.len()))
                        .ghost()
                        .selected(app.mod_view == 0)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.mod_view = 0;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("mods-tab-updates")
                        .label(format!("{} {}", tr!("Updates"), app.outdated.len()))
                        .ghost()
                        .selected(app.mod_view == 1)
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.mod_view = 1;
                            cx.notify();
                        })),
                ),
        );

        if app.mod_view == 0 {
            let packages: Vec<_> = app
                .packages
                .iter()
                .filter(|package| {
                    filter.is_empty() || package.mod_id.to_ascii_lowercase().contains(&filter)
                })
                .cloned()
                .collect();
            if packages.is_empty() {
                body = body.child(
                    ui::themed_card(cx).child(ui::empty_state(
                        OrbitIcon::Mods,
                        if app.packages.is_empty() {
                            tr!("No mods installed")
                        } else {
                            tr!("No matching packages")
                        }
                        .into_owned(),
                        if app.packages.is_empty() {
                            tr!("Browse compatible projects to build this installation.")
                        } else {
                            tr!("Try a different package name or version.")
                        }
                        .into_owned(),
                        None,
                        cx,
                    )),
                );
            } else {
                let mut list = ui::themed_card(cx).p_0().gap_0();
                for (index, package) in packages.into_iter().enumerate() {
                    if index > 0 {
                        list = list.child(ui::divider(cx));
                    }
                    let edit = package.clone();
                    let upgrade = package.mod_id.clone();
                    let remove = package.mod_id.clone();
                    let mut facts = vec![
                        package_environment_label(&package.environment),
                        tr!("%{count} remotes", count = package.remotes.len()),
                        tr!("%{count} dependencies", count = package.dependencies.len()),
                    ];
                    if package.bundled_count > 0 {
                        facts.push(tr!(
                            "%{count} bundled module(s)",
                            count = package.bundled_count
                        ));
                    }
                    list = list.child(
                        h_flex()
                            .min_w_0()
                            .px_4()
                            .py_2()
                            .gap_3()
                            .items_center()
                            .child(ui::package_icon(package.icon_path.as_deref(), cx))
                            .child(
                                v_flex()
                                    .min_w_0()
                                    .flex_1()
                                    .gap_1()
                                    .child(
                                        h_flex()
                                            .gap_2()
                                            .child(div().font_semibold().child(package.mod_id.clone()))
                                            .child(ui::neutral_pill(package.version.clone(), cx))
                                            .when(package.optional, |row| {
                                                row.child(ui::neutral_pill(
                                                    tr!("Optional").into_owned(),
                                                    cx,
                                                ))
                                            }),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(cx.theme().muted_foreground)
                                            .child(facts.join(" · ")),
                                    )
                                    ,
                            )
                            .child(
                                h_flex()
                                    .gap_1()
                                    .child(
                                        Button::new(("package-edit", index))
                                            .icon(OrbitIcon::Settings)
                                            .label(tr!("Manage").into_owned())
                                            .ghost()
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.package_editor = Some(PackageEditor::new(edit.clone()));
                                                this.load_package_versions(&edit.mod_id);
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        Button::new(("package-upgrade", index))
                                            .icon(OrbitIcon::Refresh)
                                            .ghost()
                                            .tooltip(tr!("Upgrade").into_owned())
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.upgrade_package(&upgrade);
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        Button::new(("package-remove", index))
                                            .icon(OrbitIcon::Trash)
                                            .ghost()
                                            .tooltip(tr!("Remove").into_owned())
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                this.confirmation = Some(Confirmation {
                                                    title: tr!("Remove %{package}?", package = remove),
                                                    body: tr!("The solver will show the exact package-level removal plan before writing files.").into_owned(),
                                                    action: ConfirmationAction::RemovePackage(remove.clone()),
                                                });
                                                cx.notify();
                                            })),
                                    ),
                            ),
                    );
                }
                body = body.child(list);
            }
        } else {
            body = body.child(render_updates(app, cx));
        }
        body.into_any_element()
    };

    ui::page(
        tr!("Mods").into_owned(),
        tr!("Logical packages, feasible upgrades and package remotes").into_owned(),
        actions,
        content,
        cx,
    )
}

fn package_environment_label(value: &str) -> String {
    match value {
        "client" => tr!("Client").into_owned(),
        "server" => tr!("Server").into_owned(),
        "both" => tr!("Both").into_owned(),
        "auto" => tr!("Automatic").into_owned(),
        other => other.to_string(),
    }
}

fn render_updates(app: &OrbitApp, cx: &mut Context<OrbitApp>) -> impl IntoElement {
    let mut body = v_flex().gap_3().child(
        h_flex()
            .justify_between()
            .child(ui::section_title(
                tr!("Feasible upgrades").into_owned(),
                tr!("Solver-validated candidates").into_owned(),
                cx,
            ))
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("updates-check")
                            .label(tr!("Check again").into_owned())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.run_outdated();
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("updates-all")
                            .label(tr!("Upgrade all").into_owned())
                            .primary()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.upgrade_all_packages();
                                cx.notify();
                            })),
                    ),
            ),
    );
    if !app.outdated_checked {
        return body.child(ui::themed_card(cx).child(ui::empty_state(
            OrbitIcon::Refresh,
            tr!("Not checked yet").into_owned(),
            tr!("Run the solver to find standard Pareto-maximal upgrade plans.").into_owned(),
            None,
            cx,
        )));
    }
    for (index, update) in app.outdated.iter().cloned().enumerate() {
        let package = update.mod_id.clone();
        body = body.child(
            ui::compact_card(cx).child(
                h_flex()
                    .justify_between()
                    .items_center()
                    .child(
                        v_flex()
                            .gap_1()
                            .child(div().font_semibold().child(update.mod_id))
                            .child(
                                div()
                                    .text_sm()
                                    .text_color(cx.theme().muted_foreground)
                                    .child(format!(
                                        "{}  →  {}",
                                        update.current_version, update.new_version
                                    )),
                            ),
                    )
                    .child(
                        Button::new(("update-one", index))
                            .label(tr!("Upgrade").into_owned())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.upgrade_package(&package);
                                cx.notify();
                            })),
                    ),
            ),
        );
    }
    if app.outdated.is_empty() {
        body = body.child(
            ui::compact_card(cx)
                .child(tr!("No feasible package upgrade is currently available.").into_owned()),
        );
    }
    if !app.outdated_diagnostics.is_empty() || !app.outdated_warnings.is_empty() {
        body = body.child(ui::section_title(
            tr!("Why packages did not upgrade").into_owned(),
            tr!("Preserved solver derivations").into_owned(),
            cx,
        ));
        for diagnostic in &app.outdated_diagnostics {
            body = body.child(
                ui::compact_card(cx)
                    .child(
                        h_flex()
                            .gap_2()
                            .child(ui::pill(
                                super::activity::diagnostic_kind_label(&diagnostic.kind),
                                cx.theme().warning.opacity(0.13),
                                cx.theme().warning,
                            ))
                            .child(div().font_semibold().child(diagnostic.package.clone())),
                    )
                    .child(div().text_sm().child(format!(
                        "{}  ⇢  {}",
                        diagnostic.selected_version, diagnostic.candidate_version
                    )))
                    .children(diagnostic.facts.iter().map(|fact| {
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(format!("• {fact}"))
                    })),
            );
        }
        for warning in &app.outdated_warnings {
            body = body.child(
                ui::compact_card(cx).child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().warning)
                        .child(warning.clone()),
                ),
            );
        }
    }
    body
}
