use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use eframe::egui::{
    self, Color32, CornerRadius, FontFamily, FontId, Margin, RichText, Stroke, Vec2,
};
use serde::{Deserialize, Serialize};

static DARK: AtomicBool = AtomicBool::new(false);
static ACCENT_ID: AtomicU8 = AtomicU8::new(0);

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

    const fn id(self) -> u8 {
        match self {
            Self::Indigo => 0,
            Self::Emerald => 1,
            Self::Amber => 2,
        }
    }
}

fn dark() -> bool {
    DARK.load(Ordering::Relaxed)
}

pub fn background() -> Color32 {
    if dark() {
        Color32::from_rgb(14, 17, 24)
    } else {
        Color32::from_rgb(244, 246, 250)
    }
}
pub fn sidebar() -> Color32 {
    if dark() {
        Color32::from_rgb(19, 23, 32)
    } else {
        Color32::WHITE
    }
}
pub fn surface() -> Color32 {
    if dark() {
        Color32::from_rgb(25, 30, 42)
    } else {
        Color32::WHITE
    }
}
pub fn surface_high() -> Color32 {
    if dark() {
        Color32::from_rgb(31, 37, 51)
    } else {
        Color32::from_rgb(250, 251, 253)
    }
}
pub fn control() -> Color32 {
    if dark() {
        Color32::from_rgb(36, 43, 58)
    } else {
        Color32::from_rgb(238, 241, 247)
    }
}
pub fn control_hover() -> Color32 {
    if dark() {
        Color32::from_rgb(45, 53, 70)
    } else {
        Color32::from_rgb(229, 234, 243)
    }
}
pub fn border() -> Color32 {
    if dark() {
        Color32::from_rgb(55, 64, 83)
    } else {
        Color32::from_rgb(216, 222, 233)
    }
}
pub fn text() -> Color32 {
    if dark() {
        Color32::from_rgb(239, 242, 249)
    } else {
        Color32::from_rgb(30, 37, 51)
    }
}
pub fn muted() -> Color32 {
    if dark() {
        Color32::from_rgb(155, 165, 184)
    } else {
        Color32::from_rgb(102, 112, 133)
    }
}
pub fn accent() -> Color32 {
    match (ACCENT_ID.load(Ordering::Relaxed), dark()) {
        (1, false) => Color32::from_rgb(25, 137, 105),
        (1, true) => Color32::from_rgb(54, 187, 145),
        (2, false) => Color32::from_rgb(190, 116, 20),
        (2, true) => Color32::from_rgb(224, 157, 58),
        (_, false) => Color32::from_rgb(80, 101, 230),
        (_, true) => Color32::from_rgb(129, 111, 246),
    }
}
pub fn accent_hover() -> Color32 {
    if dark() {
        accent().linear_multiply(1.12)
    } else {
        accent().linear_multiply(0.86)
    }
}
pub fn accent_soft() -> Color32 {
    match (ACCENT_ID.load(Ordering::Relaxed), dark()) {
        (1, false) => Color32::from_rgb(230, 247, 240),
        (1, true) => Color32::from_rgb(25, 62, 53),
        (2, false) => Color32::from_rgb(252, 242, 224),
        (2, true) => Color32::from_rgb(68, 50, 27),
        (_, false) => Color32::from_rgb(235, 238, 255),
        (_, true) => Color32::from_rgb(46, 42, 78),
    }
}
pub fn success() -> Color32 {
    if dark() {
        Color32::from_rgb(75, 200, 151)
    } else {
        Color32::from_rgb(30, 145, 100)
    }
}
pub fn warning() -> Color32 {
    if dark() {
        Color32::from_rgb(235, 177, 74)
    } else {
        Color32::from_rgb(194, 119, 20)
    }
}
pub fn danger() -> Color32 {
    if dark() {
        Color32::from_rgb(240, 105, 121)
    } else {
        Color32::from_rgb(210, 61, 80)
    }
}

pub fn install(ctx: &egui::Context, mode: ThemeMode, accent_theme: AccentTheme) {
    let effective = match mode {
        ThemeMode::System => ctx.system_theme().unwrap_or(egui::Theme::Light),
        ThemeMode::Light => egui::Theme::Light,
        ThemeMode::Dark => egui::Theme::Dark,
    };
    ctx.set_theme(effective);
    DARK.store(effective == egui::Theme::Dark, Ordering::Relaxed);
    ACCENT_ID.store(accent_theme.id(), Ordering::Relaxed);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = Vec2::new(9.0, 8.0);
    style.spacing.button_padding = Vec2::new(13.0, 7.0);
    style.spacing.interact_size.y = 34.0;
    style.spacing.combo_width = 180.0;
    style.spacing.text_edit_width = 240.0;
    style.animation_time = 0.12;
    let is_dark = dark();
    style.visuals.dark_mode = is_dark;
    style.visuals.panel_fill = background();
    style.visuals.window_fill = surface();
    style.visuals.window_stroke = Stroke::new(1.0, border());
    style.visuals.extreme_bg_color = if is_dark {
        Color32::from_rgb(18, 22, 31)
    } else {
        Color32::WHITE
    };
    style.visuals.faint_bg_color = surface();
    style.visuals.code_bg_color = sidebar();
    style.visuals.window_shadow = egui::epaint::Shadow {
        offset: [0, 12],
        blur: 32,
        spread: 0,
        color: Color32::from_black_alpha(if is_dark { 100 } else { 24 }),
    };
    style.visuals.popup_shadow = egui::epaint::Shadow {
        offset: [0, 8],
        blur: 24,
        spread: 0,
        color: Color32::from_black_alpha(if is_dark { 100 } else { 32 }),
    };
    style.visuals.widgets.noninteractive.weak_bg_fill = surface();
    style.visuals.widgets.noninteractive.bg_fill = surface();
    style.visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, border());
    style.visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, text());
    style.visuals.widgets.inactive.weak_bg_fill = control();
    style.visuals.widgets.inactive.bg_fill = control();
    style.visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, border());
    style.visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, text());
    style.visuals.widgets.hovered.weak_bg_fill = control_hover();
    style.visuals.widgets.hovered.bg_fill = control_hover();
    style.visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, accent());
    style.visuals.widgets.hovered.fg_stroke = Stroke::new(1.25, text());
    style.visuals.widgets.active.weak_bg_fill = accent_soft();
    style.visuals.widgets.active.bg_fill = accent_soft();
    style.visuals.widgets.active.bg_stroke = Stroke::new(1.0, accent());
    style.visuals.widgets.active.fg_stroke = Stroke::new(1.25, text());
    style.visuals.widgets.open.weak_bg_fill = control_hover();
    style.visuals.widgets.open.bg_fill = control_hover();
    style.visuals.widgets.open.bg_stroke = Stroke::new(1.0, accent());
    style.visuals.widgets.open.fg_stroke = Stroke::new(1.25, text());
    style.visuals.selection.bg_fill = accent();
    style.visuals.selection.stroke = Stroke::new(1.0, Color32::WHITE);
    style.visuals.hyperlink_color = accent();
    style.visuals.override_text_color = Some(text());
    style.visuals.window_corner_radius = CornerRadius::same(12);
    style.visuals.menu_corner_radius = CornerRadius::same(9);
    style.text_styles.insert(
        egui::TextStyle::Heading,
        FontId::new(26.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Body,
        FontId::new(14.0, FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Button,
        FontId::new(14.0, FontFamily::Proportional),
    );
    ctx.set_style_of(egui::Theme::Light, style.clone());
    ctx.set_style_of(egui::Theme::Dark, style);
}

/// Install a CJK-capable fallback when Chinese is active. The font database
/// resolves real system fonts; it never assumes one OS-specific file path.
pub fn install_language_fonts(
    ctx: &egui::Context,
    language: orbit_i18n::LanguageMode,
) -> Result<(), String> {
    let mut fonts = egui::FontDefinitions::default();
    if language.effective_locale() != "zh-CN" {
        ctx.set_fonts(fonts);
        return Ok(());
    }

    let mut database = fontdb::Database::new();
    database.load_system_fonts();
    const FAMILIES: [&str; 8] = [
        "Microsoft YaHei UI",
        "Microsoft YaHei",
        "PingFang SC",
        "Noto Sans CJK SC",
        "Noto Sans SC",
        "Source Han Sans SC",
        "WenQuanYi Micro Hei",
        "Droid Sans Fallback",
    ];
    let id = FAMILIES.iter().find_map(|family| {
        database.query(&fontdb::Query {
            families: &[fontdb::Family::Name(family)],
            weight: fontdb::Weight::NORMAL,
            stretch: fontdb::Stretch::Normal,
            style: fontdb::Style::Normal,
        })
    });
    let Some(id) = id else {
        return Err(tr!(
            "No Simplified Chinese font was found. Install Noto Sans CJK SC or Microsoft YaHei."
        )
        .into_owned());
    };
    let Some((data, face_index)) =
        database.with_face_data(id, |data, face_index| (data.to_vec(), face_index))
    else {
        return Err(tr!("The selected Simplified Chinese font could not be read.").into_owned());
    };
    const CJK_FONT: &str = "orbit-cjk";
    let mut font = egui::FontData::from_owned(data);
    font.index = face_index;
    fonts
        .font_data
        .insert(CJK_FONT.to_string(), std::sync::Arc::new(font));
    for family in [FontFamily::Proportional, FontFamily::Monospace] {
        fonts
            .families
            .entry(family)
            .or_default()
            .push(CJK_FONT.to_string());
    }
    ctx.set_fonts(fonts);
    Ok(())
}

pub fn apply_ui(ui: &mut egui::Ui) {
    let visuals = ui.visuals_mut();
    visuals.dark_mode = dark();
    visuals.panel_fill = background();
    visuals.window_fill = surface();
    visuals.extreme_bg_color = if dark() {
        Color32::from_rgb(18, 22, 31)
    } else {
        Color32::WHITE
    };
    visuals.override_text_color = Some(text());
    visuals.widgets.noninteractive.weak_bg_fill = surface();
    visuals.widgets.noninteractive.bg_fill = surface();
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, border());
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, text());
    visuals.widgets.inactive.weak_bg_fill = control();
    visuals.widgets.inactive.bg_fill = control();
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, border());
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, text());
    visuals.widgets.hovered.weak_bg_fill = control_hover();
    visuals.widgets.hovered.bg_fill = control_hover();
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, accent());
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.25, text());
    visuals.widgets.active.weak_bg_fill = accent_soft();
    visuals.widgets.active.bg_fill = accent_soft();
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, accent());
    visuals.widgets.active.fg_stroke = Stroke::new(1.25, text());
    visuals.selection.bg_fill = accent();
    visuals.selection.stroke = Stroke::new(1.0, Color32::WHITE);
}

pub fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(surface())
        .stroke(Stroke::new(1.0, border()))
        .corner_radius(11)
        .inner_margin(Margin::same(14))
}
pub fn elevated_card() -> egui::Frame {
    egui::Frame::new()
        .fill(surface_high())
        .stroke(Stroke::new(1.0, border()))
        .corner_radius(13)
        .inner_margin(Margin::same(16))
        .shadow(egui::epaint::Shadow {
            offset: [0, 4],
            blur: 16,
            spread: 0,
            color: Color32::from_black_alpha(if dark() { 72 } else { 18 }),
        })
}
pub fn primary_button(label: impl Into<String>) -> egui::Button<'static> {
    let label = label.into();
    egui::Button::new(
        RichText::new(orbit_i18n::text(&label).into_owned())
            .strong()
            .color(Color32::WHITE),
    )
    .fill(accent())
    .stroke(Stroke::NONE)
    .corner_radius(8)
    .min_size(Vec2::new(104.0, 36.0))
}
pub fn secondary_button(label: impl Into<String>) -> egui::Button<'static> {
    let label = label.into();
    egui::Button::new(
        RichText::new(orbit_i18n::text(&label).into_owned())
            .strong()
            .color(text()),
    )
    .fill(control())
    .stroke(Stroke::new(1.0, border()))
    .corner_radius(8)
    .min_size(Vec2::new(90.0, 36.0))
}
pub fn ghost_button(label: impl Into<String>) -> egui::Button<'static> {
    let label = label.into();
    egui::Button::new(RichText::new(orbit_i18n::text(&label).into_owned()).color(muted()))
        .fill(Color32::TRANSPARENT)
        .stroke(Stroke::NONE)
        .corner_radius(8)
}
pub fn danger_button(label: impl Into<String>) -> egui::Button<'static> {
    let label = label.into();
    egui::Button::new(
        RichText::new(orbit_i18n::text(&label).into_owned())
            .strong()
            .color(Color32::WHITE),
    )
    .fill(danger())
    .stroke(Stroke::NONE)
    .corner_radius(8)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum InputWidth {
    Fill,
    Form,
    Compact,
}

pub fn text_field(
    ui: &mut egui::Ui,
    value: &mut String,
    hint: &str,
    width: InputWidth,
) -> egui::Response {
    add_text_field(ui, value, hint, width, false)
}

pub fn password_field(
    ui: &mut egui::Ui,
    value: &mut String,
    hint: &str,
    width: InputWidth,
) -> egui::Response {
    add_text_field(ui, value, hint, width, true)
}

fn add_text_field(
    ui: &mut egui::Ui,
    value: &mut String,
    hint: &str,
    width: InputWidth,
    password: bool,
) -> egui::Response {
    let available = ui.available_width();
    let desired = match width {
        InputWidth::Fill => available,
        InputWidth::Form => 420.0_f32.min(available),
        InputWidth::Compact => 300.0_f32.min(available),
    };
    ui.add_sized(
        [desired, 40.0],
        egui::TextEdit::singleline(value)
            .hint_text(orbit_i18n::text(hint))
            .desired_width(desired)
            .margin(Margin::symmetric(11, 8))
            .password(password),
    )
}

pub fn section_title(ui: &mut egui::Ui, title: &str, subtitle: &str) {
    ui.label(RichText::new(orbit_i18n::text(title)).size(23.0).strong());
    ui.label(RichText::new(orbit_i18n::text(subtitle)).color(muted()));
    ui.add_space(9.0);
}
pub fn orbit_mark(ui: &mut egui::Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::splat(size), egui::Sense::hover());
    let center = rect.center();
    ui.painter()
        .circle_stroke(center, size * 0.35, Stroke::new(size * 0.09, accent()));
    ui.painter().circle_filled(
        center + Vec2::new(size * 0.27, -size * 0.24),
        size * 0.10,
        accent_hover(),
    );
    ui.painter().circle_filled(center, size * 0.10, text());
}
