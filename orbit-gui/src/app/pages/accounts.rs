use gpui::{
    Context, InteractiveElement, IntoElement, ParentElement, StatefulInteractiveElement, Styled,
    Window, div, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme, StyledExt,
    button::{Button, ButtonVariants},
    h_flex,
    input::Input,
    switch::Switch,
    v_flex,
};
use zeroize::Zeroizing;

use super::super::{AccountFlow, Confirmation, ConfirmationAction, OrbitApp};
use crate::app::components as ui;
use crate::assets::OrbitIcon;

pub(super) fn render(
    app: &mut OrbitApp,
    window: &mut Window,
    cx: &mut Context<OrbitApp>,
) -> impl IntoElement {
    let actions = if app.account_flow.is_some() {
        Button::new("accounts-back")
            .label(tr!("Back to accounts").into_owned())
            .ghost()
            .on_click(cx.listener(|this, _, _, cx| {
                this.account_flow = None;
                this.ygg_endpoint_editor_open = false;
                cx.notify();
            }))
            .into_any_element()
    } else {
        Button::new("accounts-add")
            .icon(OrbitIcon::Plus)
            .label(tr!("Add account").into_owned())
            .primary()
            .on_click(cx.listener(|this, _, _, cx| {
                this.account_flow = Some(AccountFlow::Choose);
                cx.notify();
            }))
            .into_any_element()
    };
    let content = match app.account_flow {
        Some(AccountFlow::Choose) => method_choices(app, cx).into_any_element(),
        Some(AccountFlow::Offline) => offline_form(app, window, cx).into_any_element(),
        Some(AccountFlow::YggdrasilEndpoints) => endpoints(app, window, cx).into_any_element(),
        Some(AccountFlow::YggdrasilLogin) => yggdrasil_form(app, window, cx).into_any_element(),
        None => dashboard(app, cx).into_any_element(),
    };
    ui::page(
        tr!("Accounts").into_owned(),
        tr!("Microsoft, offline and standards-based Yggdrasil identities").into_owned(),
        actions,
        content,
        cx,
    )
}

fn dashboard(app: &OrbitApp, cx: &mut Context<OrbitApp>) -> impl IntoElement {
    let load_error = app.accounts_error.clone();
    if app.accounts.is_empty() {
        if let Some(error) = load_error {
            return account_load_error(error, cx).into_any_element();
        }
        return ui::themed_card(cx)
            .child(ui::empty_state(
                OrbitIcon::Account,
                tr!("No saved accounts").into_owned(),
                tr!("Add an identity for client launch. Server installations do not require a selected player account.").into_owned(),
                None,
                cx,
            ))
            .into_any_element();
    }
    let selected_account = app
        .instance_detail
        .as_ref()
        .and_then(|detail| detail.selected_account_id.clone());
    let mut list = v_flex().gap_3();
    if let Some(error) = load_error {
        list = list.child(account_load_error(error, cx));
    }
    for (index, account) in app.accounts.iter().cloned().enumerate() {
        let use_id = account.id.clone();
        let default_id = account.id.clone();
        let refresh_id = account.id.clone();
        let remove_id = account.id.clone();
        let reauthenticate = account.authentication_state == "reauthentication-required";
        let reauth_provider = account.provider.clone();
        let reauth_provider_id = account.provider_id.clone();
        let reauth_login_name = account.login_name.clone();
        let used_here = selected_account.as_deref() == Some(account.id.as_str());
        list = list.child(
            ui::themed_card(cx).child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(ui::account_avatar(
                        account.avatar_path.as_deref(),
                        initials(&account.profile_name),
                        42.,
                        cx,
                    ))
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_1()
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(div().text_lg().font_semibold().child(account.profile_name.clone()))
                                    .when(account.is_default, |row| row.child(ui::pill(tr!("Default").into_owned(), cx.theme().primary.opacity(0.13), cx.theme().primary)))
                                    .when(used_here, |row| row.child(ui::pill(tr!("Used here").into_owned(), cx.theme().success.opacity(0.13), cx.theme().success)))
                                    .when(reauthenticate, |row| row.child(ui::pill(tr!("Sign-in expired").into_owned(), cx.theme().danger.opacity(0.13), cx.theme().danger))),
                            )
                            .child(div().text_xs().text_color(cx.theme().muted_foreground).child(format!("{}{}", provider_label(&account.provider), account.provider_id.as_deref().map(|id| format!(" · {id}")).unwrap_or_default()))),
                    )
                    .when(!used_here && app.selected_instance().is_some_and(|item| item.kind == "client"), |row| row.child(
                        Button::new(("account-use", index))
                            .label(tr!("Use here").into_owned())
                            .ghost()
                            .on_click(cx.listener(move |this, _, _, cx| { this.select_account(use_id.clone(), false); cx.notify(); })),
                    ))
                    .when(used_here, |row| row.child(
                        Button::new(("account-use-default", index))
                            .label(tr!("Use global default").into_owned())
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.clear_account_selection(false);
                                cx.notify();
                            })),
                    ))
                    .when(!account.is_default, |row| row.child(
                        Button::new(("account-default", index))
                            .label(tr!("Make default").into_owned())
                            .ghost()
                            .on_click(cx.listener(move |this, _, _, cx| { this.select_account(default_id.clone(), true); cx.notify(); })),
                    ))
                    .when(account.is_default, |row| row.child(
                        Button::new(("account-clear-default", index))
                            .label(tr!("Clear default").into_owned())
                            .ghost()
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.clear_account_selection(true);
                                cx.notify();
                            })),
                    ))
                    .when(reauthenticate, |row| row.child(
                        Button::new(("account-reauthenticate", index))
                            .label(tr!("Sign in again").into_owned())
                            .primary()
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if reauth_provider == "microsoft" {
                                    this.begin_microsoft_login();
                                } else if reauth_provider == "external-yggdrasil" {
                                    this.ygg_provider = reauth_provider_id.clone().unwrap_or_default();
                                    if let Some(login_name) = &reauth_login_name {
                                        this.inputs.ygg_username.update(cx, |input, cx| {
                                            input.set_value(login_name.clone(), window, cx)
                                        });
                                    }
                                    this.account_flow = Some(
                                        if this.yggdrasil_providers.iter().any(|provider| provider.id == this.ygg_provider) {
                                            AccountFlow::YggdrasilLogin
                                        } else {
                                            AccountFlow::YggdrasilEndpoints
                                        },
                                    );
                                }
                                cx.notify();
                            })),
                    ))
                    .when(!reauthenticate && account.provider != "offline", |row| row.child(
                        Button::new(("account-refresh", index))
                            .icon(OrbitIcon::Refresh)
                            .ghost()
                            .tooltip(tr!("Refresh account profile").into_owned())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.refresh_account(&refresh_id);
                                cx.notify();
                            })),
                    ))
                    .child(
                        Button::new(("account-remove", index))
                            .icon(OrbitIcon::Trash)
                            .ghost()
                            .tooltip(tr!("Log out and remove").into_owned())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.confirmation = Some(Confirmation {
                                    title: tr!("Log out account?").into_owned(),
                                    body: tr!("The saved session and local secret will be deleted; game files are unchanged.").into_owned(),
                                    action: ConfirmationAction::LogoutAccount(remove_id.clone()),
                                });
                                cx.notify();
                            })),
                    ),
            ),
        );
    }
    list.into_any_element()
}

fn account_load_error(error: String, cx: &mut Context<OrbitApp>) -> impl IntoElement {
    ui::themed_card(cx)
        .border_color(cx.theme().danger.opacity(0.45))
        .child(
            h_flex()
                .gap_3()
                .items_center()
                .child(ui::icon_tile(OrbitIcon::Warning, cx))
                .child(
                    v_flex()
                        .flex_1()
                        .gap_1()
                        .child(
                            div()
                                .font_semibold()
                                .child(tr!("Could not load saved accounts").into_owned()),
                        )
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(error),
                        ),
                )
                .child(
                    Button::new("accounts-retry-load")
                        .icon(OrbitIcon::Refresh)
                        .label(tr!("Retry").into_owned())
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.reload_accounts();
                            cx.notify();
                        })),
                ),
        )
}

fn method_choices(_app: &OrbitApp, cx: &mut Context<OrbitApp>) -> impl IntoElement {
    v_flex()
        .gap_4()
        .child(ui::section_title(
            tr!("Choose a sign-in method").into_owned(),
            tr!("Credentials remain owned by Launcher").into_owned(),
            cx,
        ))
        .child(
            h_flex()
                .gap_3()
                .flex_wrap()
                .child(method_card(
                    "method-microsoft",
                    "MS",
                    "Microsoft",
                    tr!("Official Minecraft account using device authorization").into_owned(),
                    cx.listener(|this, _, _, cx| {
                        this.begin_microsoft_login();
                        cx.notify();
                    }),
                    cx,
                ))
                .child(method_card(
                    "method-offline",
                    "OF",
                    tr!("Offline").into_owned(),
                    tr!("Local profile name without online authentication").into_owned(),
                    cx.listener(|this, _, _, cx| {
                        this.account_flow = Some(AccountFlow::Offline);
                        cx.notify();
                    }),
                    cx,
                ))
                .child(method_card(
                    "method-yggdrasil",
                    "YG",
                    "Yggdrasil",
                    tr!("A standards-based external authentication endpoint").into_owned(),
                    cx.listener(|this, _, _, cx| {
                        this.ygg_provider = this
                            .yggdrasil_providers
                            .first()
                            .map(|item| item.id.clone())
                            .unwrap_or_default();
                        this.ygg_endpoint_editor_open = this.yggdrasil_providers.is_empty();
                        this.account_flow = Some(AccountFlow::YggdrasilEndpoints);
                        cx.notify();
                    }),
                    cx,
                )),
        )
}

fn method_card(
    id: &'static str,
    mark: impl Into<gpui::SharedString>,
    title: impl Into<gpui::SharedString>,
    detail: impl Into<gpui::SharedString>,
    handler: impl Fn(&gpui::ClickEvent, &mut Window, &mut gpui::App) + 'static,
    cx: &gpui::App,
) -> impl IntoElement {
    div()
        .id(id)
        .w(px(238.))
        .min_h(px(164.))
        .p_4()
        .rounded_lg()
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().group_box)
        .shadow_xs()
        .cursor_pointer()
        .hover(|style| {
            style
                .bg(cx.theme().secondary)
                .border_color(cx.theme().primary.opacity(0.55))
        })
        .child(
            h_flex()
                .items_start()
                .justify_between()
                .child(
                    div()
                        .size(px(40.))
                        .rounded_lg()
                        .flex()
                        .items_center()
                        .justify_center()
                        .bg(cx.theme().primary.opacity(0.14))
                        .text_color(cx.theme().primary)
                        .font_semibold()
                        .child(mark.into()),
                )
                .child(div().text_color(cx.theme().muted_foreground).child("→")),
        )
        .child(
            v_flex()
                .mt_4()
                .gap_2()
                .child(div().text_lg().font_semibold().child(title.into()))
                .child(
                    div()
                        .text_sm()
                        .line_height(gpui::relative(1.45))
                        .text_color(cx.theme().muted_foreground)
                        .child(detail.into()),
                ),
        )
        .on_click(handler)
}

fn offline_form(
    app: &OrbitApp,
    _window: &mut Window,
    cx: &mut Context<OrbitApp>,
) -> impl IntoElement {
    let input = app.inputs.offline_name.clone();
    let read = input.clone();
    ui::themed_card(cx)
        .max_w(px(600.))
        .child(ui::field(
            tr!("Profile name").into_owned(),
            tr!("This is the player name visible to offline-mode servers.").into_owned(),
            &input,
            cx,
        ))
        .child(
            h_flex().justify_end().child(
                Button::new("offline-create")
                    .label(tr!("Create offline profile").into_owned())
                    .primary()
                    .on_click(cx.listener(move |this, _, _, cx| {
                        let name = read.read(cx).value().trim().to_string();
                        if !name.is_empty() {
                            this.create_offline_account(name);
                        }
                        cx.notify();
                    })),
            ),
        )
}

fn endpoints(app: &OrbitApp, _window: &mut Window, cx: &mut Context<OrbitApp>) -> impl IntoElement {
    let mut body = v_flex().gap_4().child(
        h_flex()
            .justify_between()
            .child(ui::section_title(
                tr!("Authentication endpoint").into_owned(),
                tr!("Choose or add an endpoint before entering credentials").into_owned(),
                cx,
            ))
            .child(
                Button::new("endpoint-new")
                    .icon(OrbitIcon::Plus)
                    .label(tr!("New endpoint").into_owned())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.ygg_endpoint_editor_open = true;
                        cx.notify();
                    })),
            ),
    );
    if app.ygg_endpoint_editor_open {
        body = body.child(endpoint_editor(app, cx));
    }
    if app.yggdrasil_providers.is_empty() {
        body = body.child(ui::themed_card(cx).child(ui::empty_state(
            OrbitIcon::Browse,
            tr!("No Yggdrasil endpoints").into_owned(),
            tr!("Add a site URL or precise API root. Launcher performs ALI discovery and metadata validation.").into_owned(),
            None,
            cx,
        )));
    }
    for (index, provider) in app.yggdrasil_providers.iter().cloned().enumerate() {
        let choose = provider.id.clone();
        let remove = provider.id.clone();
        body = body.child(
            ui::compact_card(cx).child(
                h_flex()
                    .gap_3()
                    .items_center()
                    .child(ui::icon_tile(OrbitIcon::Browse, cx))
                    .child(
                        v_flex()
                            .flex_1()
                            .gap_1()
                            .child(div().font_semibold().child(provider.id.clone()))
                            .child(div().text_xs().text_color(cx.theme().muted_foreground).child(provider.api_root.clone())),
                    )
                    .when(provider.allow_insecure_http, |row| row.child(ui::pill(tr!("HTTP allowed").into_owned(), cx.theme().warning.opacity(0.13), cx.theme().warning)))
                    .child(
                        Button::new(("endpoint-choose", index))
                            .label(tr!("Continue").into_owned())
                            .primary()
                            .on_click(cx.listener(move |this, _, _, cx| { this.ygg_provider = choose.clone(); this.account_flow = Some(AccountFlow::YggdrasilLogin); cx.notify(); })),
                    )
                    .child(
                        Button::new(("endpoint-remove", index))
                            .icon(OrbitIcon::Trash)
                            .ghost()
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.confirmation = Some(Confirmation {
                                    title: tr!("Remove authentication endpoint?").into_owned(),
                                    body: tr!("Saved accounts remain separate; new sign-ins can no longer select this endpoint.").into_owned(),
                                    action: ConfirmationAction::RemoveYggdrasilProvider(remove.clone()),
                                });
                                cx.notify();
                            })),
                    ),
            ),
        );
    }
    body
}

fn endpoint_editor(app: &OrbitApp, cx: &mut Context<OrbitApp>) -> impl IntoElement {
    let id = app.inputs.ygg_provider_id.clone();
    let root = app.inputs.ygg_api_root.clone();
    let id_read = id.clone();
    let root_read = root.clone();
    ui::themed_card(cx)
        .child(ui::field(
            tr!("Endpoint name").into_owned(),
            tr!("A stable local label").into_owned(),
            &id,
            cx,
        ))
        .child(ui::field(
            tr!("Site or API root").into_owned(),
            tr!("Launcher discovers the standard API root when given a site URL").into_owned(),
            &root,
            cx,
        ))
        .child(
            Switch::new("endpoint-http")
                .checked(app.ygg_allow_insecure_http)
                .label(tr!("Allow unencrypted HTTP (credentials can be intercepted)").into_owned())
                .on_click(cx.listener(|this, checked, _, cx| {
                    this.ygg_allow_insecure_http = *checked;
                    cx.notify();
                })),
        )
        .child(
            h_flex()
                .justify_end()
                .gap_2()
                .child(
                    Button::new("endpoint-cancel")
                        .label(tr!("Cancel").into_owned())
                        .ghost()
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.ygg_endpoint_editor_open = false;
                            cx.notify();
                        })),
                )
                .child(
                    Button::new("endpoint-save")
                        .label(tr!("Save endpoint").into_owned())
                        .primary()
                        .on_click(cx.listener(move |this, _, _, cx| {
                            let id = id_read.read(cx).value().trim().to_string();
                            let root = root_read.read(cx).value().trim().to_string();
                            if !id.is_empty() && !root.is_empty() {
                                this.add_yggdrasil_provider(id, root);
                            }
                            cx.notify();
                        })),
                ),
        )
}

fn yggdrasil_form(
    app: &OrbitApp,
    _window: &mut Window,
    cx: &mut Context<OrbitApp>,
) -> impl IntoElement {
    let username = app.inputs.ygg_username.clone();
    let password = app.inputs.ygg_password.clone();
    let profile = app.inputs.ygg_profile.clone();
    let username_read = username.clone();
    let password_read = password.clone();
    let profile_read = profile.clone();
    ui::themed_card(cx)
        .max_w(px(640.))
        .child(
            h_flex()
                .justify_between()
                .child(
                    div()
                        .text_lg()
                        .font_semibold()
                        .child(tr!("Sign in with Yggdrasil").into_owned()),
                )
                .child(ui::neutral_pill(app.ygg_provider.clone(), cx)),
        )
        .child(ui::field(
            tr!("Username").into_owned(),
            tr!("Account identifier accepted by this endpoint").into_owned(),
            &username,
            cx,
        ))
        .child(
            v_flex()
                .gap_1p5()
                .child(
                    div()
                        .text_sm()
                        .font_semibold()
                        .child(tr!("Password").into_owned()),
                )
                .child(Input::new(&password).mask_toggle()),
        )
        .child(ui::field(
            tr!("Game profile (optional)").into_owned(),
            tr!("Leave empty to use the service's default profile.").into_owned(),
            &profile,
            cx,
        ))
        .child(
            h_flex().justify_end().child(
                Button::new("ygg-login")
                    .label(tr!("Sign in").into_owned())
                    .primary()
                    .on_click(cx.listener(move |this, _, window, cx| {
                        let username = username_read.read(cx).value().trim().to_string();
                        let profile = profile_read.read(cx).value().trim().to_string();
                        let password_value = password_read.read(cx).value().to_string();
                        if !username.is_empty() && !password_value.is_empty() {
                            password_read.update(cx, |state, cx| state.set_value("", window, cx));
                            this.yggdrasil_login(username, profile, Zeroizing::new(password_value));
                        }
                        cx.notify();
                    })),
            ),
        )
}

pub(crate) fn initials(value: &str) -> String {
    value
        .split_whitespace()
        .filter_map(|part| part.chars().next())
        .take(2)
        .flat_map(char::to_uppercase)
        .collect()
}

pub(crate) fn provider_label(provider: &str) -> String {
    match provider {
        "microsoft" => "Microsoft".to_string(),
        "offline" => tr!("Offline").into_owned(),
        "yggdrasil" | "external-yggdrasil" => "Yggdrasil".to_string(),
        value => value.to_string(),
    }
}
