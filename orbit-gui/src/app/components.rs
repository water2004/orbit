use std::time::Duration;

use gpui::{
    Animation, AnimationExt, AnyElement, App, Div, ElementId, InteractiveElement, IntoElement,
    ParentElement, SharedString, Styled, StyledImage, div, img, prelude::FluentBuilder as _, px,
};
use gpui_component::{
    ActiveTheme, Icon, StyledExt,
    animation::cubic_bezier,
    h_flex,
    input::{Input, InputState},
    scroll::ScrollableElement,
    v_flex,
};

use crate::assets::OrbitIcon;

pub(super) fn page(
    title: impl Into<SharedString>,
    subtitle: impl Into<SharedString>,
    actions: impl IntoElement,
    content: impl IntoElement,
    cx: &App,
) -> Div {
    v_flex()
        .size_full()
        .min_h_0()
        .child(
            h_flex()
                .flex_shrink_0()
                .px_5()
                .pt_4()
                .pb_3()
                .items_end()
                .justify_between()
                .child(
                    v_flex()
                        .gap_1()
                        .child(div().text_2xl().font_semibold().child(title.into()))
                        .child(
                            div()
                                .text_sm()
                                .text_color(cx.theme().muted_foreground)
                                .child(subtitle.into()),
                        ),
                )
                .child(actions),
        )
        .child(
            div()
                .id("page-scroll")
                .min_h_0()
                .flex_1()
                .overflow_y_scrollbar()
                .px_5()
                .pb_5()
                .child(content),
        )
}

pub(super) fn reveal(id: impl Into<ElementId>, content: AnyElement) -> AnyElement {
    div()
        .relative()
        .size_full()
        .child(content)
        .with_animation(
            id,
            Animation::new(Duration::from_millis(180)).with_easing(cubic_bezier(0.2, 0.8, 0.2, 1.)),
            |element, delta| {
                element
                    .top(px(8.) - delta * px(8.))
                    .opacity(0.55 + delta * 0.45)
            },
        )
        .into_any_element()
}

pub(super) fn card() -> Div {
    v_flex().p_4().gap_3().rounded_lg().border_1().shadow_xs()
}

pub(super) fn themed_card(cx: &App) -> Div {
    card()
        .border_color(cx.theme().border)
        .bg(cx.theme().group_box)
}

pub(super) fn compact_card(cx: &App) -> Div {
    v_flex()
        .p_3()
        .gap_2()
        .rounded_lg()
        .border_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().group_box)
}

pub(super) fn section_title(
    title: impl Into<SharedString>,
    detail: impl Into<SharedString>,
    cx: &App,
) -> Div {
    h_flex()
        .items_baseline()
        .gap_2()
        .child(div().text_lg().font_semibold().child(title.into()))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(detail.into()),
        )
}

pub(super) fn field(
    label: impl Into<SharedString>,
    detail: impl Into<SharedString>,
    input: &gpui::Entity<InputState>,
    cx: &App,
) -> Div {
    v_flex()
        .gap_1p5()
        .child(div().text_sm().font_semibold().child(label.into()))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(detail.into()),
        )
        .child(Input::new(input).w_full().cleanable(true))
}

pub(super) fn search_input(input: &gpui::Entity<InputState>) -> Input {
    Input::new(input)
        .prefix(Icon::new(OrbitIcon::Search))
        .cleanable(true)
}

pub(super) fn pill(
    text: impl Into<SharedString>,
    background: gpui::Hsla,
    foreground: gpui::Hsla,
) -> Div {
    div()
        .px_2()
        .py_1()
        .rounded_full()
        .bg(background)
        .text_color(foreground)
        .text_xs()
        .font_medium()
        .child(text.into())
}

pub(super) fn neutral_pill(text: impl Into<SharedString>, cx: &App) -> Div {
    pill(text, cx.theme().secondary, cx.theme().secondary_foreground)
}

pub(super) fn empty_state(
    icon: OrbitIcon,
    title: impl Into<SharedString>,
    detail: impl Into<SharedString>,
    action: Option<AnyElement>,
    cx: &App,
) -> Div {
    v_flex()
        .w_full()
        .min_h(px(240.))
        .items_center()
        .justify_center()
        .gap_2()
        .child(
            div()
                .size(px(54.))
                .rounded_full()
                .flex()
                .items_center()
                .justify_center()
                .bg(cx.theme().secondary)
                .text_color(cx.theme().primary)
                .child(Icon::new(icon).size(px(26.))),
        )
        .child(div().text_lg().font_semibold().child(title.into()))
        .child(
            div()
                .max_w(px(430.))
                .text_center()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(detail.into()),
        )
        .children(action)
}

pub(super) fn metric(
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    detail: impl Into<SharedString>,
    cx: &App,
) -> Div {
    compact_card(cx)
        .flex_1()
        .min_w(px(150.))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(label.into()),
        )
        .child(div().text_2xl().font_semibold().child(value.into()))
        .child(
            div()
                .text_xs()
                .text_color(cx.theme().muted_foreground)
                .child(detail.into()),
        )
}

pub(super) fn key_value(
    label: impl Into<SharedString>,
    value: impl Into<SharedString>,
    cx: &App,
) -> Div {
    h_flex()
        .justify_between()
        .gap_4()
        .child(
            div()
                .text_sm()
                .text_color(cx.theme().muted_foreground)
                .child(label.into()),
        )
        .child(div().text_sm().font_medium().child(value.into()))
}

pub(super) fn icon_tile(icon: OrbitIcon, cx: &App) -> Div {
    div()
        .size(px(38.))
        .flex_shrink_0()
        .rounded_lg()
        .flex()
        .items_center()
        .justify_center()
        .bg(cx.theme().secondary)
        .text_color(cx.theme().primary)
        .child(Icon::new(icon).size(px(20.)))
}

pub(super) fn package_icon(icon_path: Option<&str>, cx: &App) -> Div {
    icon_tile(OrbitIcon::Mods, cx)
        .relative()
        .overflow_hidden()
        .when_some(icon_path, |frame, path| {
            frame.child(
                img(std::path::PathBuf::from(path))
                    .absolute()
                    .inset_0()
                    .size_full()
                    .object_fit(gpui::ObjectFit::Cover),
            )
        })
}

pub(super) fn account_avatar(
    avatar_path: Option<&str>,
    fallback: impl Into<SharedString>,
    size: f32,
    cx: &App,
) -> AnyElement {
    let frame = div()
        .relative()
        .size(px(size))
        .flex_shrink_0()
        .rounded_lg()
        .overflow_hidden()
        .bg(cx.theme().secondary)
        .flex()
        .items_center()
        .justify_center()
        .text_color(cx.theme().primary)
        .font_semibold()
        .child(fallback.into());
    frame
        .when_some(avatar_path, |frame, path| {
            frame.child(
                img(std::path::PathBuf::from(path))
                    .absolute()
                    .inset_0()
                    .size_full()
                    .object_fit(gpui::ObjectFit::Cover),
            )
        })
        .into_any_element()
}

pub(super) fn state_color(success: bool, cx: &App) -> gpui::Hsla {
    if success {
        cx.theme().success
    } else {
        cx.theme().danger
    }
}

pub(super) fn divider(cx: &App) -> Div {
    div().h(px(1.)).w_full().bg(cx.theme().border)
}

pub(super) fn modal_backdrop(content: impl IntoElement, cx: &App) -> impl IntoElement {
    div()
        .id("modal-backdrop")
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .p_6()
        .bg(cx.theme().overlay)
        .occlude()
        .on_scroll_wheel(|_, _, cx| cx.stop_propagation())
        .child(content)
        .with_animation(
            "modal-backdrop",
            Animation::new(Duration::from_millis(160)).with_easing(cubic_bezier(0.2, 0.8, 0.2, 1.)),
            |element, delta| element.opacity(delta),
        )
}

pub(super) fn modal(width: f32, content: impl IntoElement, cx: &App) -> impl IntoElement {
    v_flex()
        .relative()
        .w(px(width))
        .max_w_full()
        .max_h(px(640.))
        .min_h_0()
        .overflow_hidden()
        .p_5()
        .gap_4()
        .rounded_lg()
        .border_1()
        .border_color(cx.theme().border)
        .shadow_2xl()
        .bg(cx.theme().popover)
        .child(content)
        .with_animation(
            "modal-surface",
            Animation::new(Duration::from_millis(190)).with_easing(cubic_bezier(0.2, 0.8, 0.2, 1.)),
            |element, delta| {
                element
                    .top(px(12.) - delta * px(12.))
                    .opacity(0.6 + delta * 0.4)
            },
        )
}

pub(super) fn render_json_summary(value: &serde_json::Value, cx: &App) -> Div {
    let mut list = v_flex().gap_1();
    if let Some(object) = value.as_object() {
        for (key, value) in object.iter().take(12) {
            let rendered = match value {
                serde_json::Value::String(value) => value.clone(),
                serde_json::Value::Bool(value) => value.to_string(),
                serde_json::Value::Number(value) => value.to_string(),
                serde_json::Value::Array(items) => tr!("%{count} items", count = items.len()),
                serde_json::Value::Object(items) => tr!("%{count} fields", count = items.len()),
                serde_json::Value::Null => "—".to_string(),
            };
            list = list.child(key_value(key.clone(), rendered, cx));
        }
    }
    list
}
