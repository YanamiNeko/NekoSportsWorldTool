//! Shared responsive and Android input helpers.

use crate::platform::InputKind;
use crate::ui::theme;
use eframe::egui;
use std::{hash::Hash, ops::RangeInclusive};

pub const COMPACT_BREAKPOINT: f32 = 640.0;
#[allow(dead_code)]
pub const WIDE_BREAKPOINT: f32 = 1024.0;
pub const TOUCH_HEIGHT: f32 = 44.0;

/// 三档响应式断点：手机 / 平板 / 桌面。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
#[allow(dead_code)]
pub enum Layout {
    Phone,
    Tablet,
    Desktop,
}

#[allow(dead_code)]
pub fn layout_of(width: f32) -> Layout {
    if width <= COMPACT_BREAKPOINT {
        Layout::Phone
    } else if width < WIDE_BREAKPOINT {
        Layout::Tablet
    } else {
        Layout::Desktop
    }
}

#[allow(dead_code)]
pub fn layout(ui: &egui::Ui) -> Layout {
    layout_of(ui.available_width())
}

pub fn is_compact(width: f32) -> bool {
    width <= COMPACT_BREAKPOINT
}

pub fn compact_ui(ui: &egui::Ui) -> bool {
    is_compact(ui.available_width())
}

pub fn row<R>(
    ui: &mut egui::Ui,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    if compact_ui(ui) {
        ui.horizontal_wrapped(add_contents)
    } else {
        ui.horizontal(add_contents)
    }
}

#[cfg(any(target_os = "android", test))]
pub fn parse_i64_in_range(input: &str, range: RangeInclusive<i64>) -> Result<i64, String> {
    let value = input
        .trim()
        .parse::<i64>()
        .map_err(|_| "请输入整数".to_owned())?;
    if range.contains(&value) {
        Ok(value)
    } else {
        Err(format!("请输入 {} 到 {}", range.start(), range.end()))
    }
}

#[cfg(any(target_os = "android", test))]
pub fn parse_f32_in_range(input: &str, range: RangeInclusive<f32>) -> Result<f32, String> {
    let value = input
        .trim()
        .parse::<f32>()
        .map_err(|_| "请输入数字".to_owned())?;
    if value.is_finite() && range.contains(&value) {
        Ok(value)
    } else {
        Err(format!("请输入 {} 到 {}", range.start(), range.end()))
    }
}

#[cfg(target_os = "android")]
fn parse_f64_in_range(input: &str, range: RangeInclusive<f64>) -> Result<f64, String> {
    let value = input
        .trim()
        .parse::<f64>()
        .map_err(|_| "请输入数字".to_owned())?;
    if value.is_finite() && range.contains(&value) {
        Ok(value)
    } else {
        Err(format!("请输入 {} 到 {}", range.start(), range.end()))
    }
}

/// 页面定义：索引 / 完整标签 / 手机底部导航短标签 / 图标 / 分组 / 是否手机底部导航直连。
pub struct PageDef {
    pub index: usize,
    pub label: &'static str,
    /// 手机底部导航用的短标签（宽度有限，保证 5 个入口单行不换行）。
    pub short: &'static str,
    /// 导航图标（设计稿：底部导航为「图标 + 文字」竖排）。
    pub icon: &'static str,
    pub group: &'static str,
    pub mobile_root: bool,
}

pub const PAGES: [PageDef; 9] = [
    PageDef {
        index: 0,
        label: "跑步",
        short: "跑步",
        icon: "🏃",
        group: "运动",
        mobile_root: true,
    },
    PageDef {
        index: 1,
        label: "AI 运动",
        short: "AI 运动",
        icon: "🤖",
        group: "运动",
        mobile_root: true,
    },
    PageDef {
        index: 2,
        label: "运动记录",
        short: "记录",
        icon: "📋",
        group: "数据",
        mobile_root: true,
    },
    PageDef {
        index: 3,
        label: "数据",
        short: "数据",
        icon: "📊",
        group: "数据",
        mobile_root: true,
    },
    PageDef {
        index: 4,
        label: "我的",
        short: "我的",
        icon: "👤",
        group: "我的",
        mobile_root: true,
    },
    PageDef {
        index: 5,
        label: "设备信息",
        short: "设备",
        icon: "📱",
        group: "工具",
        mobile_root: false,
    },
    PageDef {
        index: 6,
        label: "运行日志",
        short: "日志",
        icon: "🧾",
        group: "工具",
        mobile_root: false,
    },
    PageDef {
        index: 7,
        label: "路网",
        short: "路网",
        icon: "🗺",
        group: "工具",
        mobile_root: false,
    },
    PageDef {
        index: 8,
        label: "关于与更新",
        short: "关于",
        icon: "ℹ",
        group: "工具",
        mobile_root: false,
    },
];

/// 桌面侧栏分组顺序。
pub const SIDE_GROUPS: [&str; 4] = ["运动", "数据", "我的", "工具"];

/// 手机底部导航首页（「我的」）。二级页（设备信息 / 运行日志 / 路网 / 关于）返回时回到这里。
pub const MOBILE_HOME: usize = 4;

pub fn page_label(index: usize) -> &'static str {
    PAGES.get(index).map(|p| p.label).unwrap_or("")
}

fn nav_item(
    ui: &mut egui::Ui,
    label: &str,
    on: bool,
    size: egui::Vec2,
) -> egui::Response {
    let fg = if on { theme::ACCENT_700 } else { theme::TEXT_DIM };
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if !ui.is_rect_visible(rect) {
        return response;
    }
    let painter = ui.painter();
    if on {
        painter.rect_filled(rect, egui::Rounding::same(theme::radius::MD), theme::ACCENT_50);
        // 设计稿 .navi.on::before：贴左缘的 3px 竖条。
        let bar = egui::Rect::from_min_size(
            egui::pos2(rect.left(), rect.top() + 7.0),
            egui::vec2(3.0, (rect.height() - 14.0).max(6.0)),
        );
        painter.rect_filled(bar, egui::Rounding::same(2.0), theme::ACCENT_500);
    }
    let font = egui::FontId::proportional(theme::FONT_BODY);
    painter.text(
        egui::pos2(rect.left() + theme::space::MD, rect.center().y),
        egui::Align2::LEFT_CENTER,
        label,
        font,
        fg,
    );
    response
}

/// 底部导航单元：图标 + 文字竖排，选中项顶部有指示条。
///
/// 对应设计稿 preview.html 的 `.p-nav .it`：
///   - `.it .ic` 是图标（display:block，字号略大、下方留 3px）；
///   - `.it.on::before` 是选中项顶部 2.5px 的强调条。
fn nav_tab(ui: &mut egui::Ui, page: &PageDef, on: bool, size: egui::Vec2) -> egui::Response {
    let fg = if on { theme::ACCENT_700 } else { theme::TEXT_DIM };
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click());
    if !ui.is_rect_visible(rect) {
        return response;
    }

    let painter = ui.painter();
    let bg = if on {
        theme::ACCENT_50
    } else {
        egui::Color32::TRANSPARENT
    };
    painter.rect_filled(rect, egui::Rounding::same(theme::radius::MD), bg);

    // 选中指示条：设计稿里贴在该单元的顶部。
    if on {
        let bar = egui::Rect::from_min_size(
            egui::pos2(rect.left() + 14.0, rect.top()),
            egui::vec2((rect.width() - 28.0).max(8.0), 2.5),
        );
        painter.rect_filled(bar, egui::Rounding::same(2.0), theme::ACCENT_500);
    }

    // 图标在上、标签在下，整块垂直居中。
    let icon_font = egui::FontId::proportional(theme::FONT_BODY + 1.0);
    let label_font = egui::FontId::proportional(theme::FONT_SMALL - 0.5);
    let icon_h = ui.fonts(|f| f.row_height(&icon_font));
    let label_h = ui.fonts(|f| f.row_height(&label_font));
    let gap = 3.0; // 设计稿 .ic 的 margin-bottom
    let top = rect.center().y - (icon_h + gap + label_h) / 2.0;

    painter.text(
        egui::pos2(rect.center().x, top),
        egui::Align2::CENTER_TOP,
        page.icon,
        icon_font,
        fg,
    );
    painter.text(
        egui::pos2(rect.center().x, top + icon_h + gap),
        egui::Align2::CENTER_TOP,
        page.short,
        label_font,
        fg,
    );

    response
}

/// 桌面分组侧栏：按「运动 / 数据 / 我的 / 工具」列出全部 9 页。
pub fn side_nav(ui: &mut egui::Ui, selected: &mut usize) -> Vec<egui::Response> {
    let mut responses = Vec::with_capacity(PAGES.len());
    let row_h = theme::CONTROL_H + 4.0;
    for group in SIDE_GROUPS {
        ui.label(
            egui::RichText::new(group)
                .size(10.5)
                .strong()
                .color(theme::TEXT_FAINT),
        );
        for page in PAGES.iter().filter(|p| p.group == group) {
            let response =
                nav_item(ui, page.label, *selected == page.index, egui::vec2(ui.available_width(), row_h));
            if response.clicked() {
                *selected = page.index;
            }
            responses.push(response);
        }
        ui.add_space(theme::space::SM);
    }
    responses
}

/// 手机底部导航：跑步 / AI 运动 / 记录 / 数据 / 我的 五项。
/// 工具页（设备信息 / 运行日志 / 路网 / 关于与更新）作为「我的」二级页，见 [`tool_entries`]。
pub fn bottom_nav(ui: &mut egui::Ui, selected: &mut usize) -> Vec<egui::Response> {
    let roots: Vec<&PageDef> = PAGES.iter().filter(|p| p.mobile_root).collect();
    let width = (ui.available_width() / roots.len() as f32).floor();
    let mut responses = Vec::with_capacity(roots.len());
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        for page in roots {
            let response = nav_tab(
                ui,
                page,
                *selected == page.index,
                egui::vec2(width, theme::TOUCH_H + 8.0),
            );
            if response.clicked() {
                *selected = page.index;
            }
            responses.push(response);
        }
    });
    responses
}

/// 「我的」二级页入口列表：设备信息 / 运行日志 / 路网 / 关于与更新。
/// 与原始实现一致，保证全部 9 页在安卓上均可达（仅入口形态由顶部分页改为底部五栏 + 二级页）。
pub fn tool_entries(ui: &mut egui::Ui, selected: &mut usize) {
    for page in PAGES.iter().filter(|p| !p.mobile_root) {
        let resp = ui.add_sized(
            [ui.available_width(), theme::TOUCH_H],
            egui::Button::new(
                egui::RichText::new(format!("{}  ›", page.label))
                    .size(theme::FONT_BODY)
                    .color(theme::TEXT),
            ),
        );
        if resp.clicked() {
            *selected = page.index;
        }
        ui.add_space(theme::space::XS);
    }
}

/// 手机二级页返回条：返回按钮 + 标题。返回 true 表示应回到「我的」。
pub fn sub_page_header(ui: &mut egui::Ui, title: &str) -> bool {
    let mut back = false;
    ui.horizontal(|ui| {
        if ui
            .add_sized(
                [72.0, theme::CONTROL_H],
                theme::ghost_btn("← 我的"),
            )
            .clicked()
        {
            back = true;
        }
        ui.label(
            egui::RichText::new(title)
                .size(theme::FONT_SUBTITLE)
                .strong()
                .color(theme::TEXT),
        );
    });
    ui.add_space(theme::space::SM);
    back
}

#[cfg(target_os = "android")]
fn input_width(ui: &egui::Ui, desired: f32) -> f32 {
    desired.min(ui.available_width().max(80.0))
}

#[cfg(target_os = "android")]
fn native_text_button(
    ui: &mut egui::Ui,
    id: egui::Id,
    value: &mut String,
    kind: InputKind,
    desired_width: f32,
) -> egui::Response {
    if let Some(edited) = crate::platform::take_edited_text(id.value()) {
        *value = edited;
    }
    let shown = match kind {
        InputKind::Password if value.is_empty() => "点击输入密码".to_owned(),
        InputKind::Password => "●".repeat(value.chars().count().min(16)),
        _ if value.is_empty() => "点击输入".to_owned(),
        _ => value.clone(),
    };
    let response = ui.add_sized(
        [input_width(ui, desired_width), TOUCH_HEIGHT],
        egui::Button::new(shown),
    );
    if response.clicked() {
        crate::platform::edit_text(id.value(), value, kind);
    }
    response
}

pub fn text_edit(
    ui: &mut egui::Ui,
    id_source: impl Hash,
    value: &mut String,
    kind: InputKind,
    desired_width: f32,
) -> egui::Response {
    let id = ui.make_persistent_id(id_source);
    #[cfg(target_os = "android")]
    {
        native_text_button(ui, id, value, kind, desired_width)
    }
    #[cfg(not(target_os = "android"))]
    {
        let mut edit = egui::TextEdit::singleline(value)
            .id(id)
            .desired_width(desired_width);
        if matches!(kind, InputKind::Password) {
            edit = edit.password(true);
        }
        ui.add(edit)
    }
}

#[cfg(target_os = "android")]
fn show_input_error(ui: &mut egui::Ui, id: egui::Id) {
    if let Some(error) = ui.ctx().data(|data| data.get_temp::<String>(id)) {
        ui.colored_label(crate::ui::theme::err(), error);
    }
}

#[cfg(target_os = "android")]
fn set_input_error(ctx: &egui::Context, id: egui::Id, error: Option<String>) {
    ctx.data_mut(|data| match error {
        Some(error) => data.insert_temp(id, error),
        None => data.remove::<String>(id),
    });
}

#[cfg(any(target_os = "android", test))]
fn numeric_control_shell<R>(
    ui: &mut egui::Ui,
    width: f32,
    add_contents: impl FnOnce(&mut egui::Ui) -> R,
) -> egui::InnerResponse<R> {
    let width = width.min(ui.max_rect().width().max(0.0));
    ui.allocate_ui_with_layout(
        egui::vec2(width, TOUCH_HEIGHT),
        egui::Layout::top_down(egui::Align::Min),
        |ui| {
            ui.set_min_width(width);
            ui.set_max_width(width);
            add_contents(ui)
        },
    )
}

pub fn drag_i64(
    ui: &mut egui::Ui,
    id_source: impl Hash,
    value: &mut i64,
    range: RangeInclusive<i64>,
    speed: f64,
    prefix: &str,
    suffix: &str,
) -> egui::Response {
    #[cfg(target_os = "android")]
    {
        let _ = speed;
        let id = ui.make_persistent_id(id_source);
        let error_id = id.with("error");
        let inner = numeric_control_shell(ui, 120.0, |ui| {
            if let Some(edited) = crate::platform::take_edited_text(id.value()) {
                match parse_i64_in_range(&edited, range.clone()) {
                    Ok(parsed) => {
                        *value = parsed;
                        set_input_error(ui.ctx(), error_id, None);
                    }
                    Err(error) => set_input_error(ui.ctx(), error_id, Some(error)),
                }
            }
            let shown = format!("{prefix}{value}{suffix}");
            let response = ui.add_sized([120.0, TOUCH_HEIGHT], egui::Button::new(shown));
            if response.clicked() {
                crate::platform::edit_text(id.value(), &value.to_string(), InputKind::Integer);
            }
            show_input_error(ui, error_id);
            response
        });
        inner.inner
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = id_source;
        ui.add(
            egui::DragValue::new(value)
                .range(range)
                .speed(speed)
                .prefix(prefix)
                .suffix(suffix),
        )
    }
}

pub fn drag_f32(
    ui: &mut egui::Ui,
    id_source: impl Hash,
    value: &mut f32,
    range: RangeInclusive<f32>,
    speed: f64,
    decimals: usize,
    suffix: &str,
) -> egui::Response {
    #[cfg(target_os = "android")]
    {
        let _ = speed;
        let id = ui.make_persistent_id(id_source);
        let error_id = id.with("error");
        let inner = numeric_control_shell(ui, 120.0, |ui| {
            if let Some(edited) = crate::platform::take_edited_text(id.value()) {
                match parse_f32_in_range(&edited, range.clone()) {
                    Ok(parsed) => {
                        *value = parsed;
                        set_input_error(ui.ctx(), error_id, None);
                    }
                    Err(error) => set_input_error(ui.ctx(), error_id, Some(error)),
                }
            }
            let shown = format!("{:.*}{suffix}", decimals, *value);
            let response = ui.add_sized([120.0, TOUCH_HEIGHT], egui::Button::new(shown));
            if response.clicked() {
                crate::platform::edit_text(id.value(), &value.to_string(), InputKind::Decimal);
            }
            show_input_error(ui, error_id);
            response
        });
        inner.inner
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = id_source;
        let _ = decimals;
        ui.add(
            egui::DragValue::new(value)
                .range(range)
                .speed(speed)
                .suffix(suffix),
        )
    }
}

pub fn drag_f64(
    ui: &mut egui::Ui,
    id_source: impl Hash,
    value: &mut f64,
    range: RangeInclusive<f64>,
    speed: f64,
    decimals: usize,
) -> egui::Response {
    #[cfg(target_os = "android")]
    {
        let _ = speed;
        let id = ui.make_persistent_id(id_source);
        let error_id = id.with("error");
        let inner = numeric_control_shell(ui, 150.0, |ui| {
            if let Some(edited) = crate::platform::take_edited_text(id.value()) {
                match parse_f64_in_range(&edited, range.clone()) {
                    Ok(parsed) => {
                        *value = parsed;
                        set_input_error(ui.ctx(), error_id, None);
                    }
                    Err(error) => set_input_error(ui.ctx(), error_id, Some(error)),
                }
            }
            let shown = format!("{:.*}", decimals, *value);
            let response = ui.add_sized([150.0, TOUCH_HEIGHT], egui::Button::new(shown));
            if response.clicked() {
                crate::platform::edit_text(id.value(), &value.to_string(), InputKind::Decimal);
            }
            show_input_error(ui, error_id);
            response
        });
        inner.inner
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = id_source;
        let _ = range;
        ui.add(
            egui::DragValue::new(value)
                .speed(speed)
                .fixed_decimals(decimals),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui;

    #[test]
    fn compact_breakpoint_covers_phone_widths_but_not_desktop() {
        assert!(is_compact(360.0));
        assert!(is_compact(412.0));
        assert!(!is_compact(880.0));
    }

    #[test]
    fn integer_input_rejects_invalid_or_out_of_range_without_a_replacement() {
        assert_eq!(parse_i64_in_range(" 23 ", 0..=23), Ok(23));
        assert!(parse_i64_in_range("24", 0..=23).is_err());
        assert!(parse_i64_in_range("abc", 0..=23).is_err());
    }

    #[test]
    fn decimal_input_rejects_non_finite_and_out_of_range_values() {
        assert_eq!(parse_f32_in_range(" 3.25 ", 0.5..=20.0), Ok(3.25));
        assert!(parse_f32_in_range("NaN", 0.5..=20.0).is_err());
        assert!(parse_f32_in_range("21", 0.5..=20.0).is_err());
    }

    #[test]
    fn layout_tiers_cover_phone_tablet_desktop() {
        assert_eq!(layout_of(360.0), Layout::Phone);
        assert_eq!(layout_of(640.0), Layout::Phone);
        assert_eq!(layout_of(800.0), Layout::Tablet);
        assert_eq!(layout_of(1024.0), Layout::Desktop);
        assert_eq!(layout_of(1440.0), Layout::Desktop);
    }

    #[test]
    fn bottom_nav_fits_phone_widths_with_touch_targets() {
        for width in [320.0, 360.0, 412.0] {
            let ctx = egui::Context::default();
            let mut rects = Vec::new();
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 720.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut selected = 0;
                    rects = bottom_nav(ui, &mut selected)
                        .into_iter()
                        .map(|response| response.rect)
                        .collect();
                });
            });
            assert_eq!(rects.len(), 5, "width={width}");
            assert!(rects
                .iter()
                .all(|rect| rect.is_finite() && rect.width() > 0.0));
            assert!(
                rects.iter().all(|rect| rect.max.x <= width + 0.5),
                "width={width}: {rects:?}"
            );
        }
    }

    #[test]
    fn side_nav_lists_every_page_within_desktop_widths() {
        for width in [1024.0, 1440.0] {
            let ctx = egui::Context::default();
            let mut rects = Vec::new();
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 720.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let mut selected = 0;
                    rects = side_nav(ui, &mut selected)
                        .into_iter()
                        .map(|response| response.rect)
                        .collect();
                });
            });
            assert_eq!(rects.len(), 9, "width={width}");
            assert!(rects
                .iter()
                .all(|rect| rect.is_finite() && rect.width() > 0.0));
            assert!(
                rects.iter().all(|rect| rect.max.x <= width + 0.5),
                "width={width}: {rects:?}"
            );
        }
    }

    #[test]
    fn responsive_parameter_row_wraps_without_invalid_or_overflowing_rectangles() {
        for width in [360.0, 412.0, 880.0] {
            let ctx = egui::Context::default();
            let mut rects = Vec::new();
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 720.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    row(ui, |ui| {
                        for (index, control_width) in
                            [110.0, 120.0, 90.0, 120.0].into_iter().enumerate()
                        {
                            rects.push(
                                ui.add_sized(
                                    [control_width, TOUCH_HEIGHT],
                                    egui::Button::new(format!("参数 {index}")),
                                )
                                .rect,
                            );
                        }
                    });
                });
            });

            assert_eq!(rects.len(), 4, "width={width}");
            assert!(rects
                .iter()
                .all(|rect| rect.is_finite() && rect.is_positive()));
            assert!(
                rects.iter().all(|rect| rect.max.x <= width + 0.5),
                "width={width}: {rects:?}"
            );
        }
    }

    #[test]
    fn numeric_errors_expand_rows_without_overlapping_following_controls() {
        for width in [320.0, 360.0, 412.0] {
            let ctx = egui::Context::default();
            let mut rectangles = Vec::new();
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 720.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    row(ui, |ui| {
                        for index in 0..3 {
                            let control = numeric_control_shell(ui, 120.0, |ui| {
                                let button = ui
                                    .add_sized([120.0, TOUCH_HEIGHT], egui::Button::new("360"))
                                    .rect;
                                let error = ui
                                    .colored_label(
                                        egui::Color32::RED,
                                        if index == 0 {
                                            "Please enter a valid value between 10 and 200"
                                        } else {
                                            "请输入 10 到 200"
                                        },
                                    )
                                    .rect;
                                (button, error)
                            });
                            assert!(control.response.rect.contains_rect(control.inner.0));
                            assert!(control.response.rect.contains_rect(control.inner.1));
                            rectangles.push(control.response.rect);
                        }
                    });
                    rectangles.push(ui.button("Next row").rect);
                });
            });
            for (index, rectangle) in rectangles.iter().enumerate() {
                assert!(
                    rectangle.max.x <= width + 0.5,
                    "width={width}: {rectangles:?}"
                );
                for other in &rectangles[index + 1..] {
                    assert!(
                        !rectangle.intersects(*other),
                        "width={width}: {rectangles:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn wrapped_range_controls_reserve_width_before_the_parent_places_them() {
        for width in [360.0, 412.0] {
            let ctx = egui::Context::default();
            let mut rects = Vec::new();
            let raw = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(width, 720.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    row(ui, |ui| {
                        rects.push(ui.label("配速范围（秒/km）：").rect);
                        let first = numeric_control_shell(ui, 120.0, |ui| {
                            ui.add_sized([120.0, TOUCH_HEIGHT], egui::Button::new("360"))
                        });
                        rects.push(first.inner.rect);
                        rects.push(ui.label("至").rect);
                        let second = numeric_control_shell(ui, 120.0, |ui| {
                            ui.add_sized([120.0, TOUCH_HEIGHT], egui::Button::new("480"))
                        });
                        rects.push(second.inner.rect);
                    });
                });
            });

            assert!(rects
                .iter()
                .all(|rect| rect.is_finite() && rect.is_positive()));
            assert!(
                rects.iter().all(|rect| rect.max.x <= width + 0.5),
                "width={width}: {rects:?}"
            );
        }
    }
}
