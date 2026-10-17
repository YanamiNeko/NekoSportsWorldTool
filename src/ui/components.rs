//! 通用组件库：卡片 / 分区标题 / 统计块 / 徽标 / 通知条 / 分段控件 / 按钮 / 键值表。
//!
//! 页面只组合这些组件，不再手写 Frame / 圆角 / 间距。作为共享组件库，
//! 个别档位可能暂无调用方，故整模块关闭 dead_code 检查。
#![allow(dead_code)]

use super::theme;
use eframe::egui;
use egui::{Color32, RichText, Rounding, Stroke};

/// 语义色调：决定徽标 / 通知条的前景与浅底。
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Ok,
    Warn,
    Err,
    Info,
    Neutral,
}

impl Tone {
    fn colors(self) -> (Color32, Color32) {
        match self {
            Tone::Ok => (theme::OK, theme::OK_BG),
            Tone::Warn => (theme::WARN, theme::WARN_BG),
            Tone::Err => (theme::ERR, theme::ERR_BG),
            Tone::Info => (theme::ACCENT_700, theme::ACCENT_50),
            Tone::Neutral => (theme::TEXT_DIM, theme::SURFACE_SUNKEN),
        }
    }
}

/// 卡片外框：白底 + 细描边 + 大圆角 + 柔和投影。
pub fn card_frame() -> egui::Frame {
    egui::Frame::none()
        .fill(theme::SURFACE)
        .stroke(Stroke::new(1.0_f32, theme::BORDER))
        .rounding(Rounding::same(theme::radius::LG))
        .inner_margin(egui::Margin::same(theme::space::LG))
        .shadow(theme::card_shadow())
}

/// 卡片。
pub fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> egui::InnerResponse<R> {
    card_frame().show(ui, add)
}

/// 带标题的卡片。
pub fn card_titled(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    card(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.label(
            RichText::new(title)
                .size(theme::FONT_SUBTITLE)
                .strong()
                .color(theme::TEXT),
        );
        ui.add_space(theme::space::MD);
        add(ui);
    });
    ui.add_space(theme::space::MD);
}

/// 页面标题 + 副标题。
pub fn page_header(ui: &mut egui::Ui, title: &str, subtitle: Option<&str>) {
    ui.label(
        RichText::new(title)
            .size(theme::FONT_TITLE)
            .strong()
            .color(theme::TEXT),
    );
    if let Some(sub) = subtitle {
        ui.label(
            RichText::new(sub)
                .size(theme::FONT_SMALL)
                .color(theme::TEXT_DIM),
        );
    }
    ui.add_space(theme::space::MD);
}

/// 卡片内的小节标题。
pub fn section_title(ui: &mut egui::Ui, text: &str) {
    ui.label(
        RichText::new(text)
            .size(theme::FONT_SMALL)
            .strong()
            .color(theme::TEXT_DIM),
    );
}

/// 一条通知：语义浅底 + 同色文字。
pub fn notice(ui: &mut egui::Ui, tone: Tone, text: &str) {
    let (fg, bg) = tone.colors();
    egui::Frame::none()
        .fill(bg)
        .rounding(Rounding::same(theme::radius::MD))
        .inner_margin(egui::Margin::symmetric(
            theme::space::SM + 2.0,
            theme::space::SM,
        ))
        .show(ui, |ui| {
            let w = ui.available_width();
            ui.set_width(w);
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new("•")
                        .size(theme::FONT_SMALL)
                        .color(fg)
                        .strong(),
                );
                ui.label(RichText::new(text).size(theme::FONT_SMALL).color(fg));
            });
        });
    ui.add_space(theme::space::XS);
}

/// 徽标（状态小胶囊）。
pub fn chip(ui: &mut egui::Ui, text: &str, tone: Tone) {
    let (fg, bg) = tone.colors();
    egui::Frame::none()
        .fill(bg)
        .rounding(Rounding::same(999.0))
        .inner_margin(egui::Margin::symmetric(theme::space::SM + 1.0, theme::space::XS - 1.0))
        .show(ui, |ui| {
            ui.label(
                RichText::new(text)
                    .size(theme::FONT_SMALL)
                    .strong()
                    .color(fg),
            );
        });
}

/// 统计块：数值 + 说明，用于「本次方案」等横排。
pub fn stat_tile(ui: &mut egui::Ui, value: &str, label: &str) {
    egui::Frame::none()
        .fill(theme::SURFACE_SUNKEN)
        .rounding(Rounding::same(theme::radius::MD))
        .inner_margin(egui::Margin::symmetric(
            theme::space::SM + 2.0,
            theme::space::SM,
        ))
        .show(ui, |ui| {
            ui.vertical(|ui| {
                ui.label(
                    RichText::new(value)
                        .size(theme::FONT_SUBTITLE)
                        .strong()
                        .color(theme::ACCENT_700),
                );
                ui.label(
                    RichText::new(label)
                        .size(theme::FONT_SMALL)
                        .color(theme::TEXT_DIM),
                );
            });
        });
}

/// 一行统计块。
pub fn stat_row(ui: &mut egui::Ui, tiles: &[(&str, &str)]) {
    ui.horizontal_wrapped(|ui| {
        for (value, label) in tiles {
            stat_tile(ui, value, label);
        }
    });
}

/// 分段控件：凹陷底 + 选中项白底描边。返回是否有变化。
pub fn segmented(ui: &mut egui::Ui, selected: &mut usize, items: &[&str]) -> bool {
    let mut changed = false;
    egui::Frame::none()
        .fill(theme::SURFACE_SUNKEN)
        .rounding(Rounding::same(theme::radius::MD))
        .inner_margin(egui::Margin::same(3.0))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = theme::space::XS;
                for (index, item) in items.iter().enumerate() {
                    let on = *selected == index;
                    let text = if on {
                        RichText::new(*item)
                            .size(theme::FONT_SMALL)
                            .strong()
                            .color(theme::ACCENT_700)
                    } else {
                        RichText::new(*item)
                            .size(theme::FONT_SMALL)
                            .color(theme::TEXT_DIM)
                    };
                    let response = ui.add(
                        egui::Button::new(text)
                            .fill(if on {
                                theme::SURFACE
                            } else {
                                Color32::TRANSPARENT
                            })
                            .stroke(if on {
                                Stroke::new(1.0_f32, theme::BORDER_STRONG)
                            } else {
                                Stroke::NONE
                            })
                            .rounding(Rounding::same(theme::radius::SM)),
                    );
                    if response.clicked() {
                        *selected = index;
                        changed = true;
                    }
                }
            });
        });
    changed
}

/// 整行宽度的按钮。
pub fn wide(
    ui: &mut egui::Ui,
    enabled: bool,
    button: egui::Button<'static>,
    height: f32,
) -> egui::Response {
    let width = ui.available_width();
    ui.add_enabled_ui(enabled, |ui| ui.add_sized([width, height], button))
        .inner
}

/// 键值表（详情页信息）。
pub fn key_values(ui: &mut egui::Ui, id: &str, rows: &[(String, String)]) {
    ui.push_id(id, |ui| {
        for (key, value) in rows {
            egui::Frame::none()
                .fill(theme::SURFACE)
                .stroke(Stroke::new(1.0_f32, theme::BORDER))
                .rounding(Rounding::same(theme::radius::MD))
                .inner_margin(egui::Margin::symmetric(
                    theme::space::SM + 2.0,
                    theme::space::SM - 2.0,
                ))
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                        ui.label(
                            RichText::new(key)
                                .size(theme::FONT_SMALL)
                                .color(theme::TEXT_DIM),
                        );
                        ui.label(
                            RichText::new(value)
                                .size(theme::FONT_BODY)
                                .color(theme::TEXT),
                        );
                    });
                });
            ui.add_space(theme::space::SM);
        }
    });
}

/// 进度条。
pub fn meter(ui: &mut egui::Ui, fraction: f32) {
    let fraction = fraction.clamp(0.0, 1.0);
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 16.0), egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(
        rect,
        Rounding::same(theme::radius::SM),
        theme::SURFACE_SUNKEN,
    );
    if fraction > 0.0 {
        let mut fill = rect;
        fill.set_width(rect.width() * fraction);
        painter.rect_filled(fill, Rounding::same(theme::radius::SM), theme::ACCENT_500);
    }
}

/// 加载中。
pub fn loading(ui: &mut egui::Ui, text: &str) {
    ui.horizontal(|ui| {
        ui.spinner();
        ui.label(
            RichText::new(text)
                .size(theme::FONT_SMALL)
                .color(theme::TEXT_DIM),
        );
    });
}

/// 空态。
pub fn empty(ui: &mut egui::Ui, text: &str) {
    ui.centered_and_justified(|ui| {
        ui.label(
            RichText::new(text)
                .size(theme::FONT_SMALL)
                .color(theme::TEXT_FAINT),
        );
    });
}

/// 分隔线。
pub fn divider(ui: &mut egui::Ui) {
    ui.add_space(theme::space::SM);
    ui.separator();
    ui.add_space(theme::space::SM);
}
