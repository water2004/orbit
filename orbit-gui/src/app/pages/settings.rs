use gpui::{Context, IntoElement, ParentElement, Styled, Window, div, px};
use gpui_component::{
    ActiveTheme, Selectable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    v_flex,
};

use super::super::OrbitApp;
use crate::app::components as ui;
use crate::assets::OrbitIcon;
use crate::theme::{AccentTheme, ThemeMode};

pub(super) fn render(
    app: &mut OrbitApp,
    _window: &mut Window,
    cx: &mut Context<OrbitApp>,
) -> impl IntoElement {
    let orbit_input = app.inputs.orbit_binary.clone();
    let launcher_input = app.inputs.launcher_binary.clone();
    let orbit_read = orbit_input.clone();
    let launcher_read = launcher_input.clone();
    let orbit_browse = orbit_input.clone();
    let launcher_browse = launcher_input.clone();

    let move_input = app.inputs.minecraft_move_destination.clone();
    let move_read = move_input.clone();
    let move_browse = move_input.clone();
    let managed_directory = app
        .minecraft_directory
        .as_ref()
        .map(|directory| directory.directory.display().to_string())
        .unwrap_or_else(|| tr!("Loading…").into_owned());
    let managed_directory_mode = app
        .minecraft_directory
        .as_ref()
        .map(|directory| {
            if directory.explicit {
                tr!("Custom location").into_owned()
            } else {
                tr!("Platform default").into_owned()
            }
        })
        .unwrap_or_default();

    let launcher_config_value = app.inputs.launcher_config_value.clone();
    let launcher_config_read = launcher_config_value.clone();
    let mut launcher_keys = h_flex().gap_2().flex_wrap();
    for (index, entry) in app.launcher_config.clone().into_iter().enumerate() {
        let key = entry.key.clone();
        let label = if entry.explicit {
            tr!("%{key} · custom", key = entry.key)
        } else {
            tr!("%{key} · default", key = entry.key)
        };
        launcher_keys = launcher_keys.child(
            Button::new(("launcher-setting", index))
                .label(label)
                .selected(app.selected_launcher_config.as_deref() == Some(entry.key.as_str()))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.select_launcher_config(key.clone(), window, cx);
                    cx.notify();
                })),
        );
    }
    let launcher_selected = app
        .selected_launcher_config
        .as_deref()
        .and_then(|key| app.launcher_config.iter().find(|entry| entry.key == key))
        .cloned();

    let orbit_config_value = app.inputs.orbit_config_value.clone();
    let orbit_config_read = orbit_config_value.clone();
    let mut orbit_keys = h_flex().gap_2().flex_wrap();
    for (index, entry) in app.orbit_config.clone().into_iter().enumerate() {
        let key = entry.key.clone();
        orbit_keys = orbit_keys.child(
            Button::new(("orbit-setting", index))
                .label(entry.key.clone())
                .selected(app.selected_orbit_config.as_deref() == Some(entry.key.as_str()))
                .on_click(cx.listener(move |this, _, window, cx| {
                    this.select_orbit_config(key.clone(), window, cx);
                    cx.notify();
                })),
        );
    }
    let orbit_selected = app
        .selected_orbit_config
        .as_deref()
        .and_then(|key| app.orbit_config.iter().find(|entry| entry.key == key))
        .cloned();

    let mut content = v_flex()
        .w_full()
        .max_w(px(1040.))
        .gap_4()
        .child(ui::section_title(
            tr!("Language").into_owned(),
            tr!("Shared by GUI and every CLI subprocess").into_owned(),
            cx,
        ))
        .child(
            ui::compact_card(cx).child(
                h_flex().gap_2().flex_wrap().children(
                    orbit_i18n::LanguageMode::ALL.into_iter().enumerate().map(
                        |(index, language)| {
                            Button::new(("language", index))
                                .label(language.label().into_owned())
                                .selected(app.preferences.language == language)
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.set_language(language);
                                    cx.notify();
                                }))
                        },
                    ),
                ),
            ),
        )
        .child(ui::section_title(
            tr!("Appearance").into_owned(),
            tr!("Theme mode and accent are persisted independently").into_owned(),
            cx,
        ))
        .child(
            ui::compact_card(cx)
                .child(h_flex().gap_2().flex_wrap().children(
                    ThemeMode::ALL.into_iter().enumerate().map(|(index, mode)| {
                        Button::new(("theme", index))
                            .label(mode.label().into_owned())
                            .selected(app.preferences.theme_mode == mode)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.set_theme(mode, window, cx);
                                cx.notify();
                            }))
                    }),
                ))
                .child(ui::divider(cx))
                .child(
                    h_flex().gap_2().flex_wrap().children(
                        AccentTheme::ALL
                            .into_iter()
                            .enumerate()
                            .map(|(index, accent)| {
                                Button::new(("accent", index))
                                    .label(accent.label().into_owned())
                                    .selected(app.preferences.accent_theme == accent)
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.set_accent(accent, window, cx);
                                        cx.notify();
                                    }))
                            }),
                    ),
                ),
        )
        .child(ui::section_title(
            tr!("CLI programs").into_owned(),
            tr!("Exact paths only; Orbit GUI never scans PATH or reads business files")
                .into_owned(),
            cx,
        ))
        .child(
            ui::themed_card(cx)
                .child(
                    v_flex()
                        .gap_1p5()
                        .child(div().text_sm().font_semibold().child("Orbit"))
                        .child(
                            h_flex()
                                .gap_2()
                                .child(Input::new(&orbit_input).flex_1())
                                .child(
                                    Button::new("orbit-path-browse")
                                        .label(tr!("Browse").into_owned())
                                        .on_click(cx.listener(move |_, _, window, cx| {
                                            if let Some(path) = rfd::FileDialog::new().pick_file() {
                                                orbit_browse.update(cx, |state, cx| {
                                                    state.set_value(
                                                        path.display().to_string(),
                                                        window,
                                                        cx,
                                                    )
                                                });
                                            }
                                        })),
                                ),
                        ),
                )
                .child(
                    v_flex()
                        .gap_1p5()
                        .child(div().text_sm().font_semibold().child("Orbit Launcher"))
                        .child(
                            h_flex()
                                .gap_2()
                                .child(Input::new(&launcher_input).flex_1())
                                .child(
                                    Button::new("launcher-path-browse")
                                        .label(tr!("Browse").into_owned())
                                        .on_click(cx.listener(move |_, _, window, cx| {
                                            if let Some(path) = rfd::FileDialog::new().pick_file() {
                                                launcher_browse.update(cx, |state, cx| {
                                                    state.set_value(
                                                        path.display().to_string(),
                                                        window,
                                                        cx,
                                                    )
                                                });
                                            }
                                        })),
                                ),
                        ),
                )
                .child(
                    h_flex().justify_end().child(
                        Button::new("paths-save")
                            .icon(OrbitIcon::Check)
                            .label(tr!("Save and refresh").into_owned())
                            .primary()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                let orbit = orbit_read.read(cx).value().trim().to_string();
                                let launcher = launcher_read.read(cx).value().trim().to_string();
                                this.save_binary_paths(orbit, launcher);
                                cx.notify();
                            })),
                    ),
                ),
        );

    if app.preferences.launcher_binary.is_file() {
        content = content
            .child(ui::section_title(
                tr!("Minecraft directory").into_owned(),
                tr!("One managed repository; every client has an isolated versions directory")
                    .into_owned(),
                cx,
            ))
            .child(
                ui::themed_card(cx)
                    .child(ui::key_value(
                        tr!("Current location").into_owned(),
                        managed_directory,
                        cx,
                    ))
                    .child(ui::key_value(
                        tr!("Location policy").into_owned(),
                        managed_directory_mode,
                        cx,
                    ))
                    .child(ui::divider(cx))
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                tr!("Moving relocates the complete repository and updates every registered client atomically. Server directories are not moved.").into_owned(),
                            ),
                    )
                    .child(
                        h_flex()
                            .gap_2()
                            .child(Input::new(&move_input).flex_1().cleanable(true))
                            .child(
                                Button::new("minecraft-directory-browse")
                                    .label(tr!("Browse").into_owned())
                                    .on_click(cx.listener(move |_, _, window, cx| {
                                        if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                            move_browse.update(cx, |state, cx| {
                                                state.set_value(
                                                    path.display().to_string(),
                                                    window,
                                                    cx,
                                                )
                                            });
                                        }
                                    })),
                            )
                            .child(
                                Button::new("minecraft-directory-move")
                                    .label(tr!("Move repository").into_owned())
                                    .primary()
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        let destination =
                                            move_read.read(cx).value().trim().to_string();
                                        this.move_minecraft_directory(destination);
                                        cx.notify();
                                    })),
                            ),
                    ),
            )
            .child(ui::section_title(
                tr!("Launcher configuration").into_owned(),
                tr!("Read and written through orbit-launcher config").into_owned(),
                cx,
            ))
            .child(
                ui::themed_card(cx)
                    .child(launcher_keys)
                    .child(ui::divider(cx))
                    .children(launcher_selected.map(|entry| {
                        v_flex()
                            .gap_2()
                            .child(ui::key_value(
                                tr!("Selected setting").into_owned(),
                                entry.key,
                                cx,
                            ))
                            .child(Input::new(&launcher_config_value).w_full().cleanable(true))
                            .child(
                                h_flex()
                                    .justify_end()
                                    .gap_2()
                                    .child(
                                        Button::new("launcher-config-reset")
                                            .label(tr!("Restore default").into_owned())
                                            .on_click(cx.listener(|this, _, _, cx| {
                                                this.unset_launcher_config();
                                                cx.notify();
                                            })),
                                    )
                                    .child(
                                        Button::new("launcher-config-save")
                                            .label(tr!("Apply").into_owned())
                                            .primary()
                                            .on_click(cx.listener(move |this, _, _, cx| {
                                                let value = launcher_config_read
                                                    .read(cx)
                                                    .value()
                                                    .trim()
                                                    .to_string();
                                                this.set_launcher_config(value);
                                                cx.notify();
                                            })),
                                    ),
                            )
                    })),
            );
    }

    content = content.child(ui::section_title(
        tr!("Orbit configuration").into_owned(),
        tr!("Available only when the Orbit executable is installed").into_owned(),
        cx,
    ));
    if app.preferences.orbit_binary.is_file() {
        let path = app
            .orbit_config_path
            .as_ref()
            .map(|path| path.display().to_string())
            .unwrap_or_default();
        content = content.child(
            ui::themed_card(cx)
                .child(ui::key_value(
                    tr!("Configuration file").into_owned(),
                    path,
                    cx,
                ))
                .child(orbit_keys)
                .child(ui::divider(cx))
                .children(orbit_selected.map(|entry| {
                    let detail = if entry.sensitive {
                        tr!("Sensitive value; leave blank until replacing it").into_owned()
                    } else {
                        tr!("Value type: %{kind}", kind = entry.value_type)
                    };
                    v_flex()
                        .gap_2()
                        .child(ui::key_value(
                            tr!("Selected setting").into_owned(),
                            entry.key,
                            cx,
                        ))
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(detail),
                        )
                        .child(Input::new(&orbit_config_value).w_full().cleanable(true))
                        .child(
                            h_flex()
                                .justify_end()
                                .gap_2()
                                .child(
                                    Button::new("orbit-config-reset")
                                        .label(tr!("Restore default").into_owned())
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.unset_orbit_config();
                                            cx.notify();
                                        })),
                                )
                                .child(
                                    Button::new("orbit-config-save")
                                        .label(tr!("Apply").into_owned())
                                        .primary()
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            let value = orbit_config_read
                                                .read(cx)
                                                .value()
                                                .trim()
                                                .to_string();
                                            this.set_orbit_config(value);
                                            cx.notify();
                                        })),
                                ),
                        )
                })),
        );
    } else {
        content = content.child(
            ui::compact_card(cx)
                .child(div().font_semibold().child(tr!("Orbit is not installed").into_owned()))
                .child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(
                            tr!("Install Orbit or select its exact executable above to manage its global configuration.").into_owned(),
                        ),
                ),
        );
    }

    content = content.child(
        ui::compact_card(cx)
            .child(
                div()
                    .font_semibold()
                    .child(tr!("Native interface stack").into_owned()),
            )
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(
                        tr!("GPUI is Apache-2.0 software from Zed Industries. Orbit uses the Apache-2.0 gpui-component control library by Longbridge; it is not a Zed-owned component set.").into_owned(),
                    ),
            ),
    );

    ui::page(
        tr!("Settings").into_owned(),
        tr!("Presentation, managed storage, and strict CLI-backed configuration").into_owned(),
        div(),
        content,
        cx,
    )
}
