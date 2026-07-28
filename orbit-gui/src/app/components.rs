use gpui::{
    AnyElement, App, Div, InteractiveElement, IntoElement, ParentElement, SharedString, Styled,
    Window, div, px,
};
use gpui_component::{
    ActiveTheme, Icon, StyledExt, h_flex,
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

pub(super) fn modal_backdrop(content: impl IntoElement, cx: &App) -> Div {
    div()
        .absolute()
        .inset_0()
        .flex()
        .items_center()
        .justify_center()
        .p_6()
        .bg(cx.theme().overlay)
        .child(content)
}

pub(super) fn modal(width: f32, content: impl IntoElement, cx: &App) -> Div {
    v_flex()
        .w(px(width))
        .max_h_full()
        .p_5()
        .gap_4()
        .rounded_lg()
        .border_1()
        .border_color(cx.theme().border)
        .shadow_2xl()
        .bg(cx.theme().popover)
        .child(content)
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

#[allow(dead_code)]
pub(super) fn _render(_window: &mut Window) {}
