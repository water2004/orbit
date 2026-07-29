use gpui::{Context, IntoElement, ParentElement, Styled, StyledImage, Window, div, img, px};
use gpui_component::{
    ActiveTheme, StyledExt,
    button::{Button, ButtonVariants},
    h_flex, v_flex,
};

use super::super::{OrbitApp, SearchState};
use crate::app::components as ui;
use crate::assets::OrbitIcon;

pub(super) fn render(
    app: &mut OrbitApp,
    _window: &mut Window,
    cx: &mut Context<OrbitApp>,
) -> impl IntoElement {
    let query_input = app.inputs.search_query.clone();
    let actions = h_flex()
        .w(px(430.))
        .gap_2()
        .child(ui::search_input(&query_input).flex_1())
        .child(
            Button::new("discover-search")
                .icon(OrbitIcon::Search)
                .label(tr!("Search").into_owned())
                .primary()
                .loading(matches!(app.search_state, SearchState::Running))
                .on_click(cx.listener(move |this, _, _, cx| {
                    let query = query_input.read(cx).value().trim().to_string();
                    if !query.is_empty() {
                        this.search_catalog(query);
                        cx.notify();
                    }
                })),
        );

    let content = if app.selected_instance().is_none() {
        ui::themed_card(cx).child(ui::empty_state(
            OrbitIcon::Runtime,
            tr!("No installation selected").into_owned(),
            tr!("Select an installation so catalog search can filter by its exact Minecraft and Loader versions.").into_owned(),
            None,
            cx,
        )).into_any_element()
    } else {
        match &app.search_state {
            SearchState::Idle => ui::themed_card(cx).child(ui::empty_state(
                OrbitIcon::Browse,
                tr!("Browse compatible projects").into_owned(),
                tr!("Search Modrinth and configured CurseForge catalogs without treating provider slugs as package identity.").into_owned(),
                None,
                cx,
            )).into_any_element(),
            SearchState::Running => ui::themed_card(cx).child(ui::empty_state(
                OrbitIcon::Search,
                tr!("Searching mod catalogs").into_owned(),
                tr!("Results will appear as providers answer.").into_owned(),
                None,
                cx,
            )).into_any_element(),
            SearchState::Failed(message) => ui::themed_card(cx).child(ui::empty_state(
                OrbitIcon::Warning,
                tr!("Search failed").into_owned(),
                message.clone(),
                None,
                cx,
            )).into_any_element(),
            SearchState::Completed if app.search_results.is_empty() => ui::themed_card(cx).child(ui::empty_state(
                OrbitIcon::Search,
                tr!("No projects found").into_owned(),
                tr!("Try a broader project name or verify the selected installation metadata.").into_owned(),
                None,
                cx,
            )).into_any_element(),
            SearchState::Completed => {
                let mut results = v_flex().gap_3();
                if app.search_truncated {
                    results = results.child(
                        ui::compact_card(cx).child(
                            div().text_sm().text_color(cx.theme().warning).child(
                                tr!("The provider returned a truncated result set; refine the query for more precise matches.").into_owned(),
                            ),
                        ),
                    );
                }
                for (index, result) in app.search_results.iter().cloned().enumerate() {
                    let add = result.clone();
                    let icon_path = result
                        .icon_url
                        .as_deref()
                        .and_then(|url| app.remote_images.path(url));
                    let icon = icon_path.map_or_else(
                        || ui::icon_tile(OrbitIcon::Mods, cx).into_any_element(),
                        |path| {
                            div()
                                .relative()
                                .size(px(48.))
                                .flex_shrink_0()
                                .rounded_lg()
                                .overflow_hidden()
                                .bg(cx.theme().secondary)
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(gpui_component::Icon::new(OrbitIcon::Mods).size(px(20.)))
                                .child(
                                    img(path.to_path_buf())
                                        .absolute()
                                        .inset_0()
                                        .size_full()
                                        .object_fit(gpui::ObjectFit::Cover),
                                )
                                .into_any_element()
                        },
                    );
                    let compatibility = result.compatible.map_or_else(
                        || tr!("Compatibility unknown").into_owned(),
                        |value| if value { tr!("Compatible") } else { tr!("Not compatible") }.into_owned(),
                    );
                    results = results.child(
                        ui::themed_card(cx).child(
                            h_flex()
                                .min_w_0()
                                .gap_3()
                                .items_start()
                                .child(icon)
                                .child(
                                    v_flex()
                                        .min_w_0()
                                        .flex_1()
                                        .gap_2()
                                        .child(
                                            h_flex()
                                                .gap_2()
                                                .items_center()
                                                .child(div().text_lg().font_semibold().child(result.name.clone()))
                                                .child(ui::neutral_pill(result.platform.clone(), cx))
                                                .child(ui::pill(
                                                    compatibility,
                                                    if result.compatible == Some(false) { cx.theme().danger } else { cx.theme().success }.opacity(0.13),
                                                    if result.compatible == Some(false) { cx.theme().danger } else { cx.theme().success },
                                                )),
                                        )
                                        .child(
                                            div()
                                                .text_sm()
                                                .text_color(cx.theme().muted_foreground)
                                                .max_w(px(680.))
                                                .child(result.description.clone()),
                                        )
                                        .child(
                                            h_flex()
                                                .gap_2()
                                                .flex_wrap()
                                                .child(ui::neutral_pill(format!("{} {}", tr!("Latest"), result.latest_version), cx))
                                                .child(ui::neutral_pill(format!("{} ↓", compact_number(result.downloads)), cx))
                                                .children(result.categories.iter().take(3).cloned().map(|category| ui::neutral_pill(category, cx))),
                                        ),
                                )
                                .child(
                                    Button::new(("discover-add", index))
                                        .icon(OrbitIcon::Plus)
                                        .label(tr!("Add").into_owned())
                                        .primary()
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.package_add = Some(super::super::PackageAddForm {
                                                project: add.clone(),
                                                environment: 0,
                                                optional: false,
                                                no_dependencies: false,
                                            });
                                            cx.notify();
                                        })),
                                ),
                        ),
                    );
                }
                results.into_any_element()
            }
        }
    };

    ui::page(
        tr!("Browse").into_owned(),
        tr!("Provider discovery filtered by the selected runtime").into_owned(),
        actions,
        content,
        cx,
    )
}

fn compact_number(value: u64) -> String {
    match value {
        0..=999 => value.to_string(),
        1_000..=999_999 => format!("{:.1}K", value as f64 / 1_000.),
        _ => format!("{:.1}M", value as f64 / 1_000_000.),
    }
}
