use gpui::{App, Hsla, Window, rgb};
use gpui_component::{ActiveTheme, Colorize, Theme};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThemeMode {
    #[default]
    System,
    Light,
    Dark,
}

impl ThemeMode {
    pub const ALL: [Self; 3] = [Self::System, Self::Light, Self::Dark];

    pub fn label(self) -> std::borrow::Cow<'static, str> {
        match self {
            Self::System => tr!("Follow system"),
            Self::Light => tr!("Light"),
            Self::Dark => tr!("Dark"),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AccentTheme {
    #[default]
    Indigo,
    Emerald,
    Amber,
}

impl AccentTheme {
    pub const ALL: [Self; 3] = [Self::Indigo, Self::Emerald, Self::Amber];

    pub fn label(self) -> std::borrow::Cow<'static, str> {
        match self {
            Self::Indigo => tr!("Indigo"),
            Self::Emerald => tr!("Emerald"),
            Self::Amber => tr!("Amber"),
        }
    }
}

pub fn apply(window: &mut Window, cx: &mut App, mode: ThemeMode, accent: AccentTheme) {
    match mode {
        ThemeMode::System => Theme::sync_system_appearance(Some(window), cx),
        ThemeMode::Light => Theme::change(gpui_component::ThemeMode::Light, Some(window), cx),
        ThemeMode::Dark => Theme::change(gpui_component::ThemeMode::Dark, Some(window), cx),
    }

    let dark = cx.theme().is_dark();
    let primary = match (accent, dark) {
        (AccentTheme::Indigo, false) => color(0x5065e6),
        (AccentTheme::Indigo, true) => color(0x8177f6),
        (AccentTheme::Emerald, false) => color(0x198969),
        (AccentTheme::Emerald, true) => color(0x36bb91),
        (AccentTheme::Amber, false) => color(0xbe7414),
        (AccentTheme::Amber, true) => color(0xe09d3a),
    };
    let theme = Theme::global_mut(cx);
    theme.font_size = gpui::px(14.);
    theme.radius = gpui::px(7.);
    theme.radius_lg = gpui::px(11.);
    theme.primary = primary;
    theme.primary_hover = if dark {
        primary.lighten(0.08)
    } else {
        primary.darken(0.08)
    };
    theme.primary_active = if dark {
        primary.lighten(0.13)
    } else {
        primary.darken(0.13)
    };
    theme.progress_bar = primary;
    theme.ring = primary;
    theme.link = primary;
    theme.link_hover = theme.primary_hover;
    theme.link_active = theme.primary_active;
    theme.sidebar_primary = primary;
    theme.sidebar_primary_foreground = gpui::white();
    theme.selection = primary.opacity(0.26);
    if dark {
        theme.background = color(0x11141c);
        theme.sidebar = color(0x171b25);
        theme.sidebar_border = color(0x303746);
        theme.group_box = color(0x1b202c);
        theme.secondary = color(0x252b38);
        theme.secondary_hover = color(0x303746);
        theme.border = color(0x343c4c);
        theme.muted = color(0x242a36);
    } else {
        theme.background = color(0xf4f6fa);
        theme.sidebar = gpui::white();
        theme.sidebar_border = color(0xd8dee9);
        theme.group_box = gpui::white();
        theme.secondary = color(0xeef1f7);
        theme.secondary_hover = color(0xe5eaf3);
        theme.border = color(0xd8dee9);
        theme.muted = color(0xeef1f7);
    }
    window.refresh();
}

pub fn color(value: u32) -> Hsla {
    rgb(value).into()
}
