//! 界面主题：浅色底 + 深青绿强调色的统一视觉。

use eframe::egui;
use egui::Color32;

const BG: Color32 = Color32::from_rgb(245, 248, 247);
const WIDGET: Color32 = Color32::from_rgb(255, 255, 255);
const WIDGET_HOVER: Color32 = Color32::from_rgb(232, 241, 238);
const BORDER: Color32 = Color32::from_rgb(206, 219, 214);
const ACCENT: Color32 = Color32::from_rgb(13, 148, 136);
const ACCENT_DIM: Color32 = Color32::from_rgb(9, 108, 99);
const TEXT: Color32 = Color32::from_rgb(24, 33, 31);
const TEXT_DIM: Color32 = Color32::from_rgb(88, 104, 99);

pub fn ok() -> Color32 {
    Color32::from_rgb(22, 130, 90)
}
pub fn warn() -> Color32 {
    Color32::from_rgb(180, 118, 0)
}
pub fn err() -> Color32 {
    Color32::from_rgb(196, 48, 48)
}
#[allow(dead_code)]
pub fn accent() -> Color32 {
    ACCENT
}
pub fn text_dim() -> Color32 {
    TEXT_DIM
}

pub fn plain() -> Color32 {
    Color32::from_rgb(52, 64, 60)
}
pub fn text() -> Color32 {
    TEXT
}

/// 主操作按钮：深青绿底白字。
pub fn primary_btn(text: &str) -> egui::Button<'static> {
    egui::Button::new(egui::RichText::new(text.to_owned()).color(Color32::WHITE).strong())
        .fill(ACCENT)
        .stroke(egui::Stroke::NONE)
        .rounding(egui::Rounding::same(6.0))
}

pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    let v = &mut style.visuals;
    *v = egui::Visuals::light();

    v.panel_fill = BG;
    v.window_fill = BG;
    v.extreme_bg_color = Color32::from_rgb(223, 232, 229);
    v.faint_bg_color = Color32::from_rgb(237, 244, 241);
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = ACCENT;
    v.selection.bg_fill = ACCENT;
    v.selection.stroke = egui::Stroke::new(1.0, ACCENT);
    v.window_stroke = egui::Stroke::new(1.0, BORDER);

    v.widgets.noninteractive.bg_fill = Color32::from_rgb(235, 241, 238);
    v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.1, TEXT);
    v.widgets.inactive.bg_fill = WIDGET;
    v.widgets.inactive.weak_bg_fill = WIDGET;
    v.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, TEXT_DIM);
    v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, BORDER);
    v.widgets.hovered.bg_fill = WIDGET_HOVER;
    v.widgets.hovered.weak_bg_fill = WIDGET_HOVER;
    v.widgets.hovered.fg_stroke = egui::Stroke::new(1.2, TEXT);
    v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, ACCENT);
    v.widgets.active.bg_fill = ACCENT_DIM;
    v.widgets.active.weak_bg_fill = ACCENT_DIM;
    v.widgets.active.fg_stroke = egui::Stroke::new(1.3, Color32::WHITE);
    v.widgets.open.bg_fill = WIDGET;
    v.widgets.open.weak_bg_fill = WIDGET;
    v.widgets.open.fg_stroke = egui::Stroke::new(1.2, TEXT);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.rounding = egui::Rounding::same(6.0);
    }

    for style_key in [
        egui::TextStyle::Heading,
        egui::TextStyle::Body,
        egui::TextStyle::Button,
        egui::TextStyle::Small,
    ] {
        let font_id = style.text_styles.get_mut(&style_key).unwrap();
        // 保持字号不变，只确保颜色由 override_text_color 控制
        let _ = font_id;
    }

    style.spacing.item_spacing = egui::vec2(10.0, 7.0);
    style.spacing.button_padding = egui::vec2(12.0, 5.0);
    style.spacing.menu_margin = egui::Margin::same(8.0);

    // 页面文字仅作展示：不可选中、不可复制，输入框不受影响
    style.interaction.selectable_labels = false;
    style.interaction.multi_widget_text_select = false;

    ctx.set_style(style);
}
