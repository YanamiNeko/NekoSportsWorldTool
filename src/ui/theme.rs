//! 界面主题：设计令牌（色阶 / 字号阶梯 / 间距 / 圆角）+ 全局样式安装。
//!
//! 色值、字号、间距、圆角一律从本模块取用，页面与组件不写裸数字。
//! 令牌集合对外完整暴露（含暂未使用的档位），故整模块关闭 dead_code 检查。
#![allow(dead_code)]

use eframe::egui;
use egui::{Color32, FontFamily, FontId, Rounding, Stroke, TextStyle};

// ── 主色（青绿）色阶 ────────────────────────────────────────
pub const ACCENT_50: Color32 = Color32::from_rgb(240, 253, 250);
pub const ACCENT_100: Color32 = Color32::from_rgb(204, 251, 241);
pub const ACCENT_200: Color32 = Color32::from_rgb(153, 246, 228);
pub const ACCENT_300: Color32 = Color32::from_rgb(94, 234, 212);
pub const ACCENT_500: Color32 = Color32::from_rgb(13, 148, 136);
pub const ACCENT_600: Color32 = Color32::from_rgb(13, 128, 118);
pub const ACCENT_700: Color32 = Color32::from_rgb(9, 108, 99);

// ── 中性色（底 / 面 / 线 / 字）──────────────────────────────
pub const BG: Color32 = Color32::from_rgb(244, 247, 246);
pub const SURFACE: Color32 = Color32::from_rgb(255, 255, 255);
pub const SURFACE_SUNKEN: Color32 = Color32::from_rgb(238, 243, 241);
pub const BORDER: Color32 = Color32::from_rgb(226, 233, 230);
pub const BORDER_STRONG: Color32 = Color32::from_rgb(203, 214, 210);
pub const TEXT: Color32 = Color32::from_rgb(22, 31, 29);
pub const TEXT_DIM: Color32 = Color32::from_rgb(92, 107, 102);
pub const TEXT_FAINT: Color32 = Color32::from_rgb(142, 155, 151);

// ── 语义色（前景 + 浅底）────────────────────────────────────
pub const OK: Color32 = Color32::from_rgb(15, 130, 92);
pub const OK_BG: Color32 = Color32::from_rgb(230, 248, 241);
pub const WARN: Color32 = Color32::from_rgb(172, 110, 0);
pub const WARN_BG: Color32 = Color32::from_rgb(253, 245, 227);
pub const ERR: Color32 = Color32::from_rgb(196, 48, 48);
pub const ERR_BG: Color32 = Color32::from_rgb(253, 237, 236);

// ── 字号阶梯（5 档）─────────────────────────────────────────
/// 页面主标题。
pub const FONT_TITLE: f32 = 21.0;
/// 卡片标题 / 关键数值。
pub const FONT_SUBTITLE: f32 = 15.5;
/// 正文与按钮。
pub const FONT_BODY: f32 = 14.0;
/// 说明文字。
pub const FONT_SMALL: f32 = 12.0;
/// 等宽（日志 / 数值）。
pub const FONT_MONO: f32 = 13.0;

// ── 间距（4pt 基准，5 档）───────────────────────────────────
pub mod space {
    /// 4
    pub const XS: f32 = 4.0;
    /// 8
    pub const SM: f32 = 8.0;
    /// 12
    pub const MD: f32 = 12.0;
    /// 16
    pub const LG: f32 = 16.0;
    /// 24
    pub const XL: f32 = 24.0;
}

// ── 圆角（3 档）─────────────────────────────────────────────
pub mod radius {
    /// 小控件：chip / 分段按钮
    pub const SM: f32 = 6.0;
    /// 中等：按钮 / 输入框 / 分组
    pub const MD: f32 = 10.0;
    /// 大：卡片 / 窗口
    pub const LG: f32 = 14.0;
}

// ── 触控与控件高度 ──────────────────────────────────────────
/// 桌面端标准控件高度。
pub const CONTROL_H: f32 = 30.0;
/// 触控最小可点区域。
pub const TOUCH_H: f32 = 44.0;
/// 主操作按钮高度。
pub const ACTION_H: f32 = 40.0;

pub fn ok() -> Color32 {
    OK
}
pub fn warn() -> Color32 {
    WARN
}
pub fn err() -> Color32 {
    ERR
}
pub fn accent() -> Color32 {
    ACCENT_500
}
pub fn text_dim() -> Color32 {
    TEXT_DIM
}
/// 次级正文（详情值）。
pub fn plain() -> Color32 {
    Color32::from_rgb(52, 64, 60)
}
pub fn text() -> Color32 {
    TEXT
}
pub fn surface() -> Color32 {
    SURFACE
}
pub fn border() -> Color32 {
    BORDER
}

/// 主操作按钮：青绿底白字。
pub fn primary_btn(text: &str) -> egui::Button<'static> {
    egui::Button::new(
        egui::RichText::new(text.to_owned())
            .color(Color32::WHITE)
            .size(FONT_BODY)
            .strong(),
    )
    .fill(ACCENT_500)
    .stroke(Stroke::NONE)
    .rounding(Rounding::same(radius::MD))
}

/// 次操作按钮：白底描边、青绿字。
pub fn secondary_btn(text: &str) -> egui::Button<'static> {
    egui::Button::new(
        egui::RichText::new(text.to_owned())
            .color(ACCENT_700)
            .size(FONT_BODY),
    )
    .fill(SURFACE)
    .stroke(Stroke::new(1.0_f32, ACCENT_100))
    .rounding(Rounding::same(radius::MD))
}

/// 轻量按钮：透明底、次级文字（列表行内操作）。
pub fn ghost_btn(text: &str) -> egui::Button<'static> {
    egui::Button::new(
        egui::RichText::new(text.to_owned())
            .color(TEXT_DIM)
            .size(FONT_BODY),
    )
    .fill(Color32::TRANSPARENT)
    .stroke(Stroke::NONE)
    .rounding(Rounding::same(radius::MD))
}

/// 危险操作按钮：红底白字。
pub fn danger_btn(text: &str) -> egui::Button<'static> {
    egui::Button::new(
        egui::RichText::new(text.to_owned())
            .color(Color32::WHITE)
            .size(FONT_BODY)
            .strong(),
    )
    .fill(ERR)
    .stroke(Stroke::NONE)
    .rounding(Rounding::same(radius::MD))
}

/// 卡片柔和投影（供组件库 card_frame 使用）。
///
/// 对齐设计稿 `--sh-1: 0 1px 3px rgba(22,31,29,.06)`，也就是 DESIGN.md §1.5
/// 的 elevation-1：offset(0,1) / blur 3 / alpha .06。早期实现用
/// offset(0,2) blur 10，卡片比设计稿更飘更灰。
///
/// Android 按 DESIGN.md §1.5 降级为「无阴影 + 描边」，只靠边框区分层级，
/// 因为移动端渲染开销更敏感。
pub fn card_shadow() -> egui::Shadow {
    #[cfg(target_os = "android")]
    {
        egui::Shadow {
            offset: egui::vec2(0.0, 0.0),
            blur: 0.0,
            spread: 0.0,
            color: Color32::TRANSPARENT,
        }
    }
    #[cfg(not(target_os = "android"))]
    {
        egui::Shadow {
            offset: egui::vec2(0.0, 1.0),
            blur: 3.0,
            spread: 0.0,
            color: Color32::from_black_alpha(15), // ≈ rgba(22,31,29,.06)
        }
    }
}

pub fn apply(ctx: &egui::Context) {
    let mut style = (*ctx.style()).clone();
    let v = &mut style.visuals;
    *v = egui::Visuals::light();

    v.panel_fill = BG;
    v.window_fill = SURFACE;
    v.extreme_bg_color = SURFACE_SUNKEN;
    v.faint_bg_color = Color32::from_rgb(241, 245, 243);
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = ACCENT_500;
    v.selection.bg_fill = ACCENT_500;
    v.selection.stroke = Stroke::new(1.0_f32, Color32::WHITE);
    v.window_stroke = Stroke::new(1.0_f32, BORDER);
    v.window_rounding = Rounding::same(radius::LG);
    v.menu_rounding = Rounding::same(radius::MD);
    v.window_shadow = egui::Shadow {
        offset: egui::vec2(0.0, 4.0),
        blur: 16.0,
        spread: 0.0,
        color: Color32::from_black_alpha(24),
    };
    v.popup_shadow = egui::Shadow {
        offset: egui::vec2(0.0, 4.0),
        blur: 16.0,
        spread: 0.0,
        color: Color32::from_black_alpha(28),
    };

    v.widgets.noninteractive.bg_fill = SURFACE_SUNKEN;
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.2_f32, TEXT);
    v.widgets.inactive.bg_fill = SURFACE;
    v.widgets.inactive.weak_bg_fill = SURFACE;
    v.widgets.inactive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, BORDER);
    v.widgets.hovered.bg_fill = ACCENT_50;
    v.widgets.hovered.weak_bg_fill = ACCENT_50;
    v.widgets.hovered.fg_stroke = Stroke::new(1.1_f32, ACCENT_700);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, ACCENT_200);
    v.widgets.hovered.expansion = 1.0;
    v.widgets.active.bg_fill = ACCENT_100;
    v.widgets.active.weak_bg_fill = ACCENT_100;
    v.widgets.active.fg_stroke = Stroke::new(1.3_f32, ACCENT_700);
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT_300);
    v.widgets.open.bg_fill = SURFACE;
    v.widgets.open.weak_bg_fill = SURFACE;
    v.widgets.open.fg_stroke = Stroke::new(1.2_f32, TEXT);
    v.widgets.open.bg_stroke = Stroke::new(1.0_f32, BORDER_STRONG);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.rounding = Rounding::same(radius::MD);
    }

    // 字号阶梯：标题 / 正文 / 按钮 / 说明 / 等宽，让页面有主次。
    // Android 整体 +1（DESIGN.md §1.2）：手机阅读距离更近，且 egui 在
    // Android 上换算 pixels_per_point 后小字号偏细。
    #[cfg(target_os = "android")]
    const SIZE_BUMP: f32 = 1.0;
    #[cfg(not(target_os = "android"))]
    const SIZE_BUMP: f32 = 0.0;

    for (key, size) in [
        (TextStyle::Heading, FONT_TITLE + SIZE_BUMP),
        (TextStyle::Body, FONT_BODY + SIZE_BUMP),
        (TextStyle::Button, FONT_BODY + SIZE_BUMP),
        (TextStyle::Small, FONT_SMALL + SIZE_BUMP),
        (TextStyle::Monospace, FONT_MONO + SIZE_BUMP),
    ] {
        let family = if key == TextStyle::Monospace {
            FontFamily::Monospace
        } else {
            FontFamily::Proportional
        };
        style.text_styles.insert(key, FontId::new(size, family));
    }

    style.spacing.item_spacing = egui::vec2(space::SM, space::SM);
    style.spacing.button_padding = egui::vec2(12.0, 6.0);
    style.spacing.menu_margin = egui::Margin::same(space::SM);
    style.spacing.window_margin = egui::Margin::same(space::MD);
    style.spacing.indent = space::LG;
    style.spacing.interact_size = egui::vec2(28.0, CONTROL_H);
    style.spacing.scroll.bar_width = 9.0;
    #[cfg(target_os = "android")]
    {
        style.spacing.interact_size = egui::vec2(TOUCH_H, TOUCH_H);
        style.spacing.button_padding = egui::vec2(14.0, 10.0);
    }

    // 详情页的 UUID / 链接需要能划选复制。
    style.interaction.selectable_labels = true;
    style.interaction.multi_widget_text_select = false;

    ctx.set_style(style);
}
