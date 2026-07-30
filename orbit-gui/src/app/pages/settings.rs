use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use gpui::{
    App, AppContext, Context, Entity, IntoElement, ParentElement, SharedString, Styled, Window,
    div, px,
};
use gpui_component::{
    ActiveTheme, Selectable, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::{Input, InputState},
    setting::{SettingField, SettingGroup, SettingItem, SettingPage, Settings},
    v_flex,
};

use super::super::{Confirmation, ConfirmationAction, OrbitApp};
use crate::app::components as ui;
use crate::assets::OrbitIcon;
use crate::model::{LauncherConfigEntry, OrbitConfigEntry};
use crate::theme::{AccentTheme, ThemeMode};

pub(super) fn render(
    app: &mut OrbitApp,
    _window: &mut Window,
    cx: &mut Context<OrbitApp>,
) -> impl IntoElement {
    let orbit_available = app.preferences.orbit_binary.is_file();
    let app = cx.entity();
    let pages = [
        appearance_page(&app),
        launcher_page(&app),
        java_page(&app),
        orbit_page(&app, orbit_available),
    ];

    div().size_full().child(
        Settings::new("orbit-settings")
            .sidebar_width(px(196.))
            .pages(pages),
    )
}

fn appearance_page(app: &Entity<OrbitApp>) -> SettingPage {
    SettingPage::new(tr!("General").into_owned())
        .description(
            tr!("Language, visual style, and the exact programs controlled by this thin GUI")
                .into_owned(),
        )
        .group(
            SettingGroup::new()
                .title(tr!("Presentation").into_owned())
                .item(language_item(app))
                .item(theme_item(app))
                .item(accent_item(app)),
        )
        .group(
            SettingGroup::new()
                .title(tr!("Command-line programs").into_owned())
                .description(
                    tr!("The GUI invokes these programs through their JSON protocol; it does not duplicate their business logic.").into_owned(),
                )
                .item(binary_paths_item(app)),
        )
}

fn launcher_page(app: &Entity<OrbitApp>) -> SettingPage {
    SettingPage::new(tr!("Launcher").into_owned())
        .description(
            tr!("Minecraft storage, download behavior, and installer limits")
                .into_owned(),
        )
        .group(
            SettingGroup::new()
                .title(tr!("Managed Minecraft repository").into_owned())
                .description(
                    tr!("All clients share immutable assets and libraries while each instances/<instance> directory owns its Minecraft JAR and mutable game data.").into_owned(),
                )
                .item(minecraft_directory_item(app)),
        )
        .group(
            SettingGroup::new()
                .title(tr!("Downloads and installation").into_owned())
                .items([
                    launcher_editor_item(
                        app,
                        "network.concurrency",
                        tr!("Parallel downloads").into_owned(),
                        tr!("Maximum number of artifacts transferred concurrently").into_owned(),
                        false,
                    ),
                    launcher_editor_item(
                        app,
                        "network.connect-timeout-seconds",
                        tr!("Connection timeout").into_owned(),
                        tr!("Seconds allowed while establishing an HTTP connection").into_owned(),
                        false,
                    ),
                    launcher_editor_item(
                        app,
                        "network.request-timeout-seconds",
                        tr!("Request timeout").into_owned(),
                        tr!("Seconds allowed for one metadata or artifact request").into_owned(),
                        false,
                    ),
                    launcher_editor_item(
                        app,
                        "installer.timeout-seconds",
                        tr!("Loader installer timeout").into_owned(),
                        tr!("Maximum runtime of the official Forge or NeoForge installer").into_owned(),
                        false,
                    ),
                    launcher_editor_item(
                        app,
                        "cache.max-size",
                        tr!("Artifact cache capacity").into_owned(),
                        tr!("LRU limit such as 8 GiB; cleanup runs after each command").into_owned(),
                        false,
                    ),
                ]),
        )
        .group(
            SettingGroup::new()
                .title(tr!("Independent CLI output").into_owned())
                .description(
                    tr!("These options affect orbit-launcher when it is used directly in a terminal, not this GUI.").into_owned(),
                )
                .items([
                    launcher_choice_item(
                        app,
                        "ui.progress-bar",
                        tr!("Progress display").into_owned(),
                        tr!("Terminal progress rendering mode").into_owned(),
                        "auto",
                        &[("auto", tr!("Automatic").into_owned()), ("always", tr!("Always").into_owned()), ("never", tr!("Never").into_owned())],
                    ),
                    launcher_choice_item(
                        app,
                        "ui.color",
                        tr!("Terminal color").into_owned(),
                        tr!("ANSI color policy for direct CLI use").into_owned(),
                        "auto",
                        &[("auto", tr!("Automatic").into_owned()), ("always", tr!("Always").into_owned()), ("never", tr!("Never").into_owned())],
                    ),
                ]),
        )
}

fn java_page(app: &Entity<OrbitApp>) -> SettingPage {
    SettingPage::new("Java")
        .description(tr!("Verified Mojang runtimes shared by Launcher instances").into_owned())
        .group(
            SettingGroup::new()
                .title(tr!("Managed runtimes").into_owned())
                .description(
                    tr!("Files are verified against the Launcher inventory before they are used.")
                        .into_owned(),
                )
                .item(java_inventory_item(app)),
        )
}

fn orbit_page(app: &Entity<OrbitApp>, available: bool) -> SettingPage {
    let page = SettingPage::new("Orbit").description(
        tr!("Package resolution, provider credentials, network policy, and the JAR cache")
            .into_owned(),
    );
    if !available {
        return page.group(
            SettingGroup::new()
                .title(tr!("Orbit is not installed").into_owned())
                .item(SettingItem::render(|_, _, cx| {
                    ui::empty_state(
                        OrbitIcon::Settings,
                        tr!("Orbit is not installed").into_owned(),
                        tr!("Install Orbit or select its exact executable on the General page to manage its global configuration.").into_owned(),
                        None,
                        cx,
                    )
                })),
        );
    }
    page
        .group(
            SettingGroup::new()
                .title(tr!("Resolver").into_owned())
                .items([
                    orbit_editor_item(
                        app,
                        "core.default-instance",
                        tr!("Default Orbit instance").into_owned(),
                        tr!("Instance used by standalone Orbit CLI commands when no local project is selected")
                            .into_owned(),
                        false,
                    ),
                    orbit_editor_item(
                        app,
                        "core.max-concurrent-downloads",
                        tr!("Parallel mod downloads").into_owned(),
                        tr!("Maximum concurrent provider and JAR transfers").into_owned(),
                        false,
                    ),
                    orbit_choice_item(
                        app,
                        "core.language",
                        tr!("Standalone Orbit language").into_owned(),
                        tr!("Default locale used when Orbit is called outside this GUI").into_owned(),
                        "system",
                        &[("system", tr!("Follow system").into_owned()), ("en", "English".to_string()), ("zh-CN", "简体中文".to_string())],
                    ),
                ]),
        )
        .group(
            SettingGroup::new()
                .title(tr!("Network").into_owned())
                .items([
                    orbit_editor_item(
                        app,
                        "network.timeout",
                        tr!("Network timeout").into_owned(),
                        tr!("Seconds allowed for provider requests and downloads").into_owned(),
                        false,
                    ),
                    orbit_editor_item(
                        app,
                        "network.max-retries",
                        tr!("Retry limit").into_owned(),
                        tr!("Maximum retries after a retryable network failure").into_owned(),
                        false,
                    ),
                    orbit_editor_item(
                        app,
                        "network.proxy",
                        tr!("Proxy URL").into_owned(),
                        tr!("Optional explicit network proxy used by providers").into_owned(),
                        false,
                    ),
                ]),
        )
        .group(
            SettingGroup::new()
                .title(tr!("Provider credentials").into_owned())
                .description(
                    tr!("Secrets are never displayed after loading; entering a new value replaces the saved credential.").into_owned(),
                )
                .items([
                    orbit_editor_item(
                        app,
                        "auth.curseforge-api-key",
                        tr!("CurseForge API key").into_owned(),
                        tr!("Required before the CurseForge provider can be used").into_owned(),
                        true,
                    ),
                    orbit_editor_item(
                        app,
                        "auth.modrinth-token",
                        tr!("Modrinth token").into_owned(),
                        tr!("Optional token for authenticated Modrinth requests").into_owned(),
                        true,
                    ),
                ]),
        )
        .group(
            SettingGroup::new()
                .title(tr!("JAR cache").into_owned())
                .items([
                    orbit_editor_item(
                        app,
                        "cache.dir",
                        tr!("Cache directory").into_owned(),
                        tr!("Optional absolute location for downloaded and analyzed JARs")
                            .into_owned(),
                        false,
                    ),
                    orbit_editor_item(
                        app,
                        "cache.capacity-mib",
                        tr!("Cache capacity").into_owned(),
                        tr!("LRU capacity in MiB, enforced after every command").into_owned(),
                        false,
                    ),
                ])
                .item(orbit_cache_clean_item(app)),
        )
        .group(
            SettingGroup::new()
                .title(tr!("Independent CLI output").into_owned())
                .items([
                    orbit_choice_item(
                        app,
                        "ui.progress-bar",
                        tr!("Progress display").into_owned(),
                        tr!("Terminal progress rendering mode").into_owned(),
                        "modern",
                        &[("modern", tr!("Modern").into_owned()), ("plain", tr!("Plain").into_owned()), ("off", tr!("Off").into_owned())],
                    ),
                    orbit_choice_item(
                        app,
                        "ui.color",
                        tr!("Terminal color").into_owned(),
                        tr!("ANSI color policy for direct CLI use").into_owned(),
                        "auto",
                        &[("auto", tr!("Automatic").into_owned()), ("always", tr!("Always").into_owned()), ("never", tr!("Never").into_owned())],
                    ),
                ]),
        )
}

fn orbit_cache_clean_item(app: &Entity<OrbitApp>) -> SettingItem {
    let app = app.clone();
    SettingItem::new(
        tr!("Cache maintenance").into_owned(),
        SettingField::render(move |_, _, _cx| {
            let app = app.clone();
            Button::new("orbit-cache-clean")
                .icon(OrbitIcon::Trash)
                .label(tr!("Clean cache…").into_owned())
                .danger()
                .on_click(move |_, _, cx| {
                    app.update(cx, |this, cx| {
                        this.confirmation = Some(Confirmation {
                            title: tr!("Clean the Orbit JAR cache?").into_owned(),
                            body: tr!("Downloaded and analyzed JAR cache entries will be removed. Installed instance files and configuration are not touched.").into_owned(),
                            action: ConfirmationAction::CleanOrbitCache,
                        });
                        cx.notify();
                    });
                })
        }),
    )
    .description(tr!("LRU cleanup also runs automatically after every Orbit command").into_owned())
}

fn language_item(app: &Entity<OrbitApp>) -> SettingItem {
    let app = app.clone();
    SettingItem::new(
        tr!("Language").into_owned(),
        SettingField::render(move |_, _, cx| {
            let selected = app.read(cx).preferences.language;
            h_flex()
                .gap_1()
                .children(orbit_i18n::LanguageMode::ALL.into_iter().enumerate().map(
                    |(index, language)| {
                        let app = app.clone();
                        Button::new(("setting-language", index))
                            .label(language.label().into_owned())
                            .selected(selected == language)
                            .on_click(move |_, _, cx| {
                                app.update(cx, |this, cx| {
                                    this.set_language(language);
                                    cx.notify();
                                });
                            })
                    },
                ))
        }),
    )
    .description(tr!("Used by the GUI and passed explicitly to every CLI subprocess").into_owned())
}

fn theme_item(app: &Entity<OrbitApp>) -> SettingItem {
    let app = app.clone();
    SettingItem::new(
        tr!("Color mode").into_owned(),
        SettingField::render(move |_, _, cx| {
            let selected = app.read(cx).preferences.theme_mode;
            h_flex()
                .gap_1()
                .children(ThemeMode::ALL.into_iter().enumerate().map(|(index, mode)| {
                    let app = app.clone();
                    Button::new(("setting-theme", index))
                        .label(mode.label().into_owned())
                        .selected(selected == mode)
                        .on_click(move |_, window, cx| {
                            app.update(cx, |this, cx| {
                                this.set_theme(mode, window, cx);
                                cx.notify();
                            });
                        })
                }))
        }),
    )
    .description(tr!("Follow the operating system or force a light or dark palette").into_owned())
}

fn accent_item(app: &Entity<OrbitApp>) -> SettingItem {
    let app = app.clone();
    SettingItem::new(
        tr!("Accent color").into_owned(),
        SettingField::render(move |_, _, cx| {
            let selected = app.read(cx).preferences.accent_theme;
            h_flex()
                .gap_1()
                .children(
                    AccentTheme::ALL
                        .into_iter()
                        .enumerate()
                        .map(|(index, accent)| {
                            let app = app.clone();
                            Button::new(("setting-accent", index))
                                .label(accent.label().into_owned())
                                .selected(selected == accent)
                                .on_click(move |_, window, cx| {
                                    app.update(cx, |this, cx| {
                                        this.set_accent(accent, window, cx);
                                        cx.notify();
                                    });
                                })
                        }),
                )
        }),
    )
    .description(tr!("Applied consistently to navigation, selections, and progress").into_owned())
}

fn binary_paths_item(app: &Entity<OrbitApp>) -> SettingItem {
    let app = app.clone();
    SettingItem::render(move |_, _, cx| {
        let orbit_input = app.read(cx).inputs.orbit_binary.clone();
        let launcher_input = app.read(cx).inputs.launcher_binary.clone();
        let orbit_browse = orbit_input.clone();
        let launcher_browse = launcher_input.clone();
        let orbit_read = orbit_input.clone();
        let launcher_read = launcher_input.clone();
        let save_app = app.clone();
        v_flex()
            .gap_3()
            .child(path_row("Orbit", orbit_input, orbit_browse))
            .child(path_row("Orbit Launcher", launcher_input, launcher_browse))
            .child(
                h_flex().justify_end().child(
                    Button::new("settings-save-programs")
                        .icon(OrbitIcon::Check)
                        .label(tr!("Save and refresh").into_owned())
                        .primary()
                        .on_click(move |_, _, cx| {
                            let orbit = orbit_read.read(cx).value().trim().to_string();
                            let launcher = launcher_read.read(cx).value().trim().to_string();
                            save_app.update(cx, |this, cx| {
                                this.save_binary_paths(orbit, launcher);
                                cx.notify();
                            });
                        }),
                ),
            )
    })
}

fn path_row(
    label: &'static str,
    input: Entity<InputState>,
    browse_input: Entity<InputState>,
) -> impl IntoElement {
    v_flex()
        .gap_1()
        .child(div().text_sm().font_medium().child(label))
        .child(
            h_flex().gap_2().child(Input::new(&input).flex_1()).child(
                Button::new(SharedString::from(format!(
                    "settings-browse-program:{label}"
                )))
                .label(tr!("Browse").into_owned())
                .on_click(move |_, window, cx| {
                    if let Some(path) = rfd::FileDialog::new().pick_file() {
                        browse_input.update(cx, |state, cx| {
                            state.set_value(path.display().to_string(), window, cx)
                        });
                    }
                }),
            ),
        )
}

fn minecraft_directory_item(app: &Entity<OrbitApp>) -> SettingItem {
    let app = app.clone();
    SettingItem::render(move |_, _, cx| {
        let directory = app.read(cx).minecraft_directory.clone();
        let input = app.read(cx).inputs.minecraft_move_destination.clone();
        let browse = input.clone();
        let read = input.clone();
        let move_app = app.clone();
        let current = directory
            .as_ref()
            .map(|value| value.directory.display().to_string())
            .unwrap_or_else(|| tr!("Unavailable").into_owned());
        let policy = directory
            .as_ref()
            .map(|value| {
                if value.explicit {
                    tr!("Custom location").into_owned()
                } else {
                    tr!("Platform default").into_owned()
                }
            })
            .unwrap_or_default();
        v_flex()
            .gap_3()
            .child(ui::key_value(tr!("Current location").into_owned(), current, cx))
            .child(ui::key_value(tr!("Location policy").into_owned(), policy, cx))
            .child(ui::divider(cx))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr!("Moving relocates the complete shared repository and rewrites every registered client path atomically. Server directories are unaffected.").into_owned()),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(Input::new(&input).flex_1().cleanable(true))
                    .child(
                        Button::new("settings-browse-minecraft")
                            .label(tr!("Browse").into_owned())
                            .on_click(move |_, window, cx| {
                                if let Some(path) = rfd::FileDialog::new().pick_folder() {
                                    browse.update(cx, |state, cx| {
                                        state.set_value(path.display().to_string(), window, cx)
                                    });
                                }
                            }),
                    )
                    .child(
                        Button::new("settings-move-minecraft")
                            .label(tr!("Move repository").into_owned())
                            .primary()
                            .on_click(move |_, _, cx| {
                                let destination = read.read(cx).value().trim().to_string();
                                move_app.update(cx, |this, cx| {
                                    this.move_minecraft_directory(destination);
                                    cx.notify();
                                });
                            }),
                    ),
            )
    })
}

fn java_inventory_item(app: &Entity<OrbitApp>) -> SettingItem {
    let app = app.clone();
    SettingItem::render(move |_, _, cx| {
        let runtimes = app.read(cx).java_runtimes.clone();
        if runtimes.is_empty() {
            return ui::empty_state(
                OrbitIcon::Java,
                tr!("No managed Java runtime").into_owned(),
                tr!("Launcher installs the required Java runtime when an instance is installed.")
                    .into_owned(),
                None,
                cx,
            )
            .into_any_element();
        }
        v_flex()
            .gap_2()
            .children(runtimes.into_iter().enumerate().map(|(index, runtime)| {
                let verify_id = runtime.runtime_id.clone();
                let remove_id = runtime.runtime_id.clone();
                let verify_app = app.clone();
                let remove_app = app.clone();
                ui::compact_card(cx).child(
                    h_flex()
                        .gap_3()
                        .items_center()
                        .child(ui::icon_tile(OrbitIcon::Java, cx))
                        .child(
                            v_flex()
                                .flex_1()
                                .child(
                                    div().font_semibold().child(format!(
                                        "Java {} · {}",
                                        runtime.major, runtime.version
                                    )),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(format!(
                                            "{} · {} · {} · {} · {}",
                                            super::super::controller::title_case(&runtime.provider),
                                            presentation_token(&runtime.component),
                                            presentation_token(&runtime.platform),
                                            tr!("%{count} files", count = runtime.files),
                                            super::super::controller::human_bytes(runtime.bytes)
                                        )),
                                ),
                        )
                        .child(ui::pill(
                            if runtime.verified == Some(true) {
                                tr!("Verified").into_owned()
                            } else {
                                tr!("Not verified").into_owned()
                            },
                            if runtime.verified == Some(true) {
                                cx.theme().success.opacity(0.13)
                            } else {
                                cx.theme().warning.opacity(0.13)
                            },
                            if runtime.verified == Some(true) {
                                cx.theme().success
                            } else {
                                cx.theme().warning
                            },
                        ))
                        .child(
                            Button::new(("settings-verify-java", index))
                                .label(tr!("Verify").into_owned())
                                .ghost()
                                .on_click(move |_, _, cx| {
                                    verify_app.update(cx, |this, cx| {
                                        this.verify_java_runtime(&verify_id);
                                        cx.notify();
                                    });
                                }),
                        )
                        .child(
                            Button::new(("settings-remove-java", index))
                                .icon(OrbitIcon::Trash)
                                .ghost()
                                .tooltip(tr!("Remove runtime").into_owned())
                                .on_click(move |_, _, cx| {
                                    remove_app.update(cx, |this, cx| {
                                        this.confirmation = Some(Confirmation {
                                            title: tr!("Remove managed Java runtime?").into_owned(),
                                            body: tr!("Removal is refused while any installed instance still references this runtime.").into_owned(),
                                            action: ConfirmationAction::RemoveJavaRuntime(
                                                remove_id.clone(),
                                            ),
                                        });
                                        cx.notify();
                                    });
                                }),
                        ),
                )
            }))
            .into_any_element()
    })
}

fn launcher_editor_item(
    app: &Entity<OrbitApp>,
    key: &'static str,
    title: String,
    description: String,
    masked: bool,
) -> SettingItem {
    let app = app.clone();
    SettingItem::new(
        title,
        SettingField::render(move |_, window, cx| {
            let entry = launcher_entry(app.read(cx), key);
            let value = entry
                .and_then(|entry| entry.value.clone())
                .unwrap_or_default();
            setting_editor(
                app.clone(),
                SettingOwner::Launcher,
                key,
                value,
                entry.is_some_and(|entry| entry.explicit),
                masked,
                window,
                cx,
            )
        }),
    )
    .description(description)
}

fn orbit_editor_item(
    app: &Entity<OrbitApp>,
    key: &'static str,
    title: String,
    description: String,
    masked: bool,
) -> SettingItem {
    let app = app.clone();
    SettingItem::new(
        title,
        SettingField::render(move |_, window, cx| {
            let entry = orbit_entry(app.read(cx), key);
            let value = entry
                .filter(|entry| !entry.sensitive)
                .map(OrbitConfigEntry::display_value)
                .unwrap_or_default();
            setting_editor(
                app.clone(),
                SettingOwner::Orbit,
                key,
                value,
                entry.is_some_and(|entry| entry.value.is_some()),
                masked,
                window,
                cx,
            )
        }),
    )
    .description(description)
}

fn launcher_choice_item(
    app: &Entity<OrbitApp>,
    key: &'static str,
    title: String,
    description: String,
    default: &'static str,
    options: &[(&'static str, String)],
) -> SettingItem {
    let app_for_value = app.clone();
    let app_for_set = app.clone();
    let options = options
        .iter()
        .map(|(value, label)| ((*value).into(), label.clone().into()))
        .collect();
    SettingItem::new(
        title,
        SettingField::dropdown(
            options,
            move |cx| {
                launcher_entry(app_for_value.read(cx), key)
                    .and_then(|entry| entry.value.clone())
                    .unwrap_or_else(|| default.to_string())
                    .into()
            },
            move |value: SharedString, cx| {
                app_for_set.update(cx, |this, cx| {
                    if value.as_ref() == default {
                        this.unset_launcher_config(key.to_string());
                    } else {
                        this.set_launcher_config(key.to_string(), value.to_string());
                    }
                    cx.notify();
                });
            },
        )
        .default_value(SharedString::from(default)),
    )
    .description(description)
}

fn orbit_choice_item(
    app: &Entity<OrbitApp>,
    key: &'static str,
    title: String,
    description: String,
    default: &'static str,
    options: &[(&'static str, String)],
) -> SettingItem {
    let app_for_value = app.clone();
    let app_for_set = app.clone();
    let options = options
        .iter()
        .map(|(value, label)| ((*value).into(), label.clone().into()))
        .collect();
    SettingItem::new(
        title,
        SettingField::dropdown(
            options,
            move |cx| {
                orbit_entry(app_for_value.read(cx), key)
                    .map(OrbitConfigEntry::display_value)
                    .filter(|value| !value.is_empty())
                    .unwrap_or_else(|| default.to_string())
                    .into()
            },
            move |value: SharedString, cx| {
                app_for_set.update(cx, |this, cx| {
                    if value.as_ref() == default {
                        this.unset_orbit_config(key.to_string());
                    } else {
                        this.set_orbit_config(key.to_string(), value.to_string());
                    }
                    cx.notify();
                });
            },
        )
        .default_value(SharedString::from(default)),
    )
    .description(description)
}

#[derive(Clone, Copy)]
enum SettingOwner {
    Launcher,
    Orbit,
}

#[allow(clippy::too_many_arguments)]
fn setting_editor(
    app: Entity<OrbitApp>,
    owner: SettingOwner,
    key: &'static str,
    value: String,
    explicit: bool,
    masked: bool,
    window: &mut Window,
    cx: &mut App,
) -> gpui::AnyElement {
    let mut hasher = DefaultHasher::new();
    value.hash(&mut hasher);
    explicit.hash(&mut hasher);
    let revision = hasher.finish();
    let input_state = window.use_keyed_state(
        SharedString::from(format!("setting:{key}:{revision}")),
        cx,
        |window, cx| {
            cx.new(|cx| {
                InputState::new(window, cx)
                    .default_value(value)
                    .masked(masked)
            })
        },
    );
    let input = input_state.read(cx).clone();
    let apply_input = input.clone();
    let apply_app = app.clone();
    let reset_app = app.clone();
    h_flex()
        .gap_2()
        .items_center()
        .child(
            div()
                .min_w(px(72.))
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(if explicit {
                    tr!("Custom").into_owned()
                } else {
                    tr!("Default").into_owned()
                }),
        )
        .child(Input::new(&input).w(px(230.)).cleanable(!masked))
        .child(
            Button::new(SharedString::from(format!("setting-reset:{key}")))
                .icon(OrbitIcon::Refresh)
                .ghost()
                .tooltip(tr!("Restore default").into_owned())
                .on_click(move |_, _, cx| {
                    reset_app.update(cx, |this, cx| {
                        match owner {
                            SettingOwner::Launcher => this.unset_launcher_config(key.to_string()),
                            SettingOwner::Orbit => this.unset_orbit_config(key.to_string()),
                        };
                        cx.notify();
                    });
                }),
        )
        .child(
            Button::new(SharedString::from(format!("setting-apply:{key}")))
                .icon(OrbitIcon::Check)
                .primary()
                .tooltip(tr!("Apply").into_owned())
                .on_click(move |_, _, cx| {
                    let value = apply_input.read(cx).value().trim().to_string();
                    apply_app.update(cx, |this, cx| {
                        match owner {
                            SettingOwner::Launcher => {
                                this.set_launcher_config(key.to_string(), value)
                            }
                            SettingOwner::Orbit => this.set_orbit_config(key.to_string(), value),
                        };
                        cx.notify();
                    });
                }),
        )
        .into_any_element()
}

fn launcher_entry<'a>(app: &'a OrbitApp, key: &str) -> Option<&'a LauncherConfigEntry> {
    app.launcher_config.iter().find(|entry| entry.key == key)
}

fn orbit_entry<'a>(app: &'a OrbitApp, key: &str) -> Option<&'a OrbitConfigEntry> {
    app.orbit_config.iter().find(|entry| entry.key == key)
}

fn presentation_token(value: &str) -> String {
    value
        .split(['-', '_'])
        .filter(|part| !part.is_empty())
        .map(super::super::controller::title_case)
        .collect::<Vec<_>>()
        .join(" ")
}
