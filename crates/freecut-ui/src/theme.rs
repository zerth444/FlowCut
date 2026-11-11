//! Тёмная тема FreeCut: палитра, стили egui, загрузка шрифтов.
//!
//! Визуальный референс — CapCut: глубокий тёмный фон, панельные слои,
//! один бирюзовый акцент, аккуратные скругления 6–8px.

use std::sync::Arc;

use egui::{
    Color32, Context, CornerRadius, FontData, FontDefinitions, FontFamily, FontId, Shadow, Stroke,
    TextStyle, style::WidgetVisuals,
};

/// Палитра приложения.
#[derive(Debug, Clone, Copy)]
pub struct Palette;

impl Palette {
    pub const BG: Color32 = rgb(0x121214);
    pub const BG_DEEP: Color32 = rgb(0x0D0D0F);
    pub const PANEL: Color32 = rgb(0x17171A);
    pub const PANEL_UPPER: Color32 = rgb(0x1A1A1E);
    pub const SURFACE: Color32 = rgb(0x222228);
    pub const SURFACE_HOVER: Color32 = rgb(0x2A2A31);
    pub const SURFACE_PRESSED: Color32 = rgb(0x31313A);
    pub const STROKE: Color32 = rgb(0x2C2C33);
    pub const STROKE_SOFT: Color32 = rgb(0x232329);
    pub const TEXT: Color32 = rgb(0xE9E9EE);
    pub const TEXT_DIM: Color32 = rgb(0x8B8B94);
    pub const TEXT_FAINT: Color32 = rgb(0x5C5C64);
    pub const ACCENT: Color32 = rgb(0x2FD4B5);
    pub const ACCENT_HOVER: Color32 = rgb(0x4BE4C8);
    pub const ACCENT_PRESSED: Color32 = rgb(0x23B699);
    pub const ACCENT_TEXT: Color32 = rgb(0x07241E);
    pub const DANGER: Color32 = rgb(0xE5534B);
    pub const VIDEO_CLIP: Color32 = rgb(0x3250D6);
    pub const VIDEO_CLIP_SOFT: Color32 = rgb(0x4664E8);
    pub const AUDIO_WAVE: Color32 = rgb(0x2FD4B5);
    pub const AUDIO_CLIP: Color32 = rgb(0x1E7F6E);
    pub const PLAYHEAD: Color32 = rgb(0xFF547D);
}

const FONT_SIZE_BODY: f32 = 14.0;
const FONT_SIZE_LABEL: f32 = 13.0;

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

/// Скругление со всех сторон.
#[must_use]
pub fn radius(value: u8) -> CornerRadius {
    CornerRadius::same(value)
}

/// Полупрозрачный вариант цвета для слоёв и ховеров.
#[must_use]
pub fn alpha(base: Color32, a: u8) -> Color32 {
    Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), a)
}

/// Установить шрифты и общий стиль приложения.
pub fn install(ctx: &Context) {
    load_fonts(ctx);
    ctx.all_styles_mut(apply_style);
}

fn apply_style(style: &mut egui::Style) {
    style.animation_time = 0.12;
    kill_dev_overlays(style);

    let spacing = &mut style.spacing;
    spacing.item_spacing = egui::vec2(8.0, 6.0);
    spacing.button_padding = egui::vec2(10.0, 5.0);
    spacing.icon_width = 15.0;
    spacing.icon_width_inner = 9.0;
    spacing.icon_spacing = 4.0;
    spacing.slider_width = 110.0;
    spacing.indent = 12.0;
    spacing.menu_margin = egui::Margin::same(6);
    spacing.menu_width = 240.0;

    let v = &mut style.visuals;
    apply_visuals(v);

    let set_widget = |w: WidgetVisuals, bg: Color32| {
        let mut w = w;
        w.bg_fill = bg;
        w.weak_bg_fill = bg;
        w.fg_stroke = Stroke::new(1.0, Palette::TEXT);
        w.bg_stroke = Stroke::NONE;
        w.corner_radius = radius(6);
        w
    };
    v.widgets.inactive = set_widget(v.widgets.inactive, Palette::SURFACE);
    v.widgets.hovered = set_widget(v.widgets.hovered, Palette::SURFACE_HOVER);
    v.widgets.active = set_widget(v.widgets.active, Palette::SURFACE_PRESSED);

    apply_text_sizes(style);
}

/// egui в debug-сборках по умолчанию рисует отладочные строки
/// («hovered := Id::...», списки виджетов на hover) — вырубаем их целиком.
#[cfg(debug_assertions)]
fn kill_dev_overlays(style: &mut egui::Style) {
    style.debug = egui::style::DebugOptions {
        debug_on_hover: false,
        debug_on_hover_with_all_modifiers: false,
        hover_shows_next: false,
        show_expand_width: false,
        show_expand_height: false,
        show_resize: false,
        show_interactive_widgets: false,
        show_widget_hits: false,
        ..egui::style::DebugOptions::default()
    };
}

#[cfg(not(debug_assertions))]
fn kill_dev_overlays(style: &mut egui::Style) {
    let _ = style;
}

fn apply_visuals(v: &mut egui::Visuals) {
    v.override_text_color = Some(Palette::TEXT);
    v.panel_fill = Palette::PANEL;
    v.window_fill = Palette::SURFACE;
    v.extreme_bg_color = Palette::BG_DEEP;
    v.faint_bg_color = alpha(Color32::WHITE, 5);
    v.hyperlink_color = Palette::ACCENT;

    v.selection.bg_fill = alpha(Palette::ACCENT, 70);
    v.selection.stroke = Stroke::new(1.0, Palette::ACCENT);

    v.window_corner_radius = radius(10);
    v.menu_corner_radius = radius(10);
    v.window_stroke = Stroke::new(1.0, Palette::STROKE);
    v.window_shadow = Shadow {
        offset: [0, 8],
        blur: 28,
        spread: 0,
        color: Color32::from_black_alpha(120),
    };

    for state in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        state.corner_radius = radius(6);
    }

    v.widgets.hovered.fg_stroke = Stroke::new(1.0, Palette::TEXT);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, alpha(Color32::WHITE, 26));
    v.widgets.active.fg_stroke = Stroke::new(1.0, Palette::ACCENT);

    v.text_cursor.stroke = Stroke::new(1.5, Palette::TEXT);
}

/// Шрифты: сначала Inter (системный или из ./assets), затем Segoe UI.
/// Встроенные шрифты egui остаются фолбэком, поэтому Linux/macOS работают.
fn load_fonts(ctx: &Context) {
    let mut fonts = FontDefinitions::default();

    let candidates: &[(&str, &[&str])] = &[
        (
            "app-semibold",
            &[
                "C:\\Windows\\Fonts\\Inter-SemiBold.ttf",
                "assets\\Inter-SemiBold.ttf",
            ],
        ),
        (
            "app-medium",
            &[
                "C:\\Windows\\Fonts\\Inter-Medium.ttf",
                "assets\\Inter-Medium.ttf",
            ],
        ),
        (
            "app-regular",
            &[
                "C:\\Windows\\Fonts\\Inter-Regular.ttf",
                "assets\\Inter-Regular.ttf",
            ],
        ),
        ("sys-bold", &["C:\\Windows\\Fonts\\segoeuib.ttf"]),
        ("sys-regular", &["C:\\Windows\\Fonts\\segoeui.ttf"]),
    ];

    let mut loaded: Vec<String> = Vec::new();
    for (name, paths) in candidates {
        for path in *paths {
            if let Ok(bytes) = std::fs::read(path) {
                fonts
                    .font_data
                    .insert((*name).to_owned(), Arc::new(FontData::from_owned(bytes)));
                loaded.push((*name).to_owned());
                break;
            }
        }
    }

    let has = |name: &str| loaded.iter().any(|n| n == name);
    if let Some(main) = ["app-regular", "sys-regular"].into_iter().find(|n| has(n)) {
        let bold = ["app-semibold", "app-medium", "sys-bold"]
            .into_iter()
            .find(|n| has(n));
        fonts
            .families
            .entry(FontFamily::Proportional)
            .or_default()
            .insert(0, main.to_owned());
        if let Some(bold) = bold {
            fonts
                .families
                .entry(FontFamily::Proportional)
                .or_default()
                .insert(1, bold.to_owned());
        }
    }

    ctx.set_fonts(fonts);
}

/// Типоразмеры текста.
fn apply_text_sizes(style: &mut egui::Style) {
    style.text_styles = [
        (TextStyle::Body, FontId::proportional(FONT_SIZE_BODY)),
        (TextStyle::Button, FontId::proportional(FONT_SIZE_BODY)),
        (
            TextStyle::Small,
            FontId::proportional(FONT_SIZE_LABEL - 1.0),
        ),
        (
            TextStyle::Heading,
            FontId::proportional(FONT_SIZE_BODY + 6.0),
        ),
        (
            TextStyle::Name("SectionTitle".into()),
            FontId::proportional(FONT_SIZE_BODY + 1.0),
        ),
        (
            TextStyle::Name("Title".into()),
            FontId::proportional(FONT_SIZE_BODY + 4.0),
        ),
    ]
    .into_iter()
    .collect();
}

/// Стиль заголовка секции панели («Медиатека», «Инспектор»).
#[must_use]
pub fn section_title(text: &str) -> egui::RichText {
    egui::RichText::new(text)
        .text_style(TextStyle::Name("SectionTitle".into()))
        .color(Palette::TEXT)
}
