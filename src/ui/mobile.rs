//! Shared responsive and Android input helpers.

use crate::platform::InputKind;
use eframe::egui;
use std::{hash::Hash, ops::RangeInclusive};

pub const COMPACT_BREAKPOINT: f32 = 620.0;
pub const TOUCH_HEIGHT: f32 = 44.0;

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
pub fn parse_i64_in_range(
    input: &str,
    range: RangeInclusive<i64>,
) -> Result<i64, String> {
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
pub fn parse_f32_in_range(
    input: &str,
    range: RangeInclusive<f32>,
) -> Result<f32, String> {
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
fn parse_f64_in_range(
    input: &str,
    range: RangeInclusive<f64>,
) -> Result<f64, String> {
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

pub fn tab_bar(ui: &mut egui::Ui, selected: &mut usize) -> Vec<egui::Response> {
    const TABS: [&str; 7] = ["跑步", "AI运动", "运动记录", "数据", "我的", "设备信息", "运行日志"];
    let compact = compact_ui(ui);
    let mut responses = Vec::with_capacity(TABS.len());
    if compact {
        let spacing = ui.spacing().item_spacing.x;
        let width = ((ui.available_width() - spacing * 3.0) / 4.0).max(68.0);
        ui.horizontal_wrapped(|ui| {
            for (index, label) in TABS.into_iter().enumerate() {
                let response = ui.add_sized(
                    [width, TOUCH_HEIGHT],
                    egui::SelectableLabel::new(*selected == index, label),
                );
                if response.clicked() {
                    *selected = index;
                }
                responses.push(response);
            }
        });
    } else {
        ui.horizontal(|ui| {
            for (index, label) in TABS.into_iter().enumerate() {
                responses.push(ui.selectable_value(selected, index, label));
            }
        });
    }
    responses
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
    fn tab_bar_stays_inside_offscreen_phone_and_desktop_frames() {
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
                    let mut selected = 0;
                    rects = tab_bar(ui, &mut selected)
                        .into_iter()
                        .map(|response| response.rect)
                        .collect();
                });
            });

            assert_eq!(rects.len(), 7, "width={width}");
            assert!(rects.iter().all(|rect| rect.is_finite() && rect.width() > 0.0));
            assert!(rects.iter().all(|rect| rect.max.x <= width + 0.5), "width={width}: {rects:?}");
            if is_compact(width) {
                assert!(rects.iter().all(|rect| rect.height() >= 44.0), "width={width}: {rects:?}");
            }
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
                        for (index, control_width) in [110.0, 120.0, 90.0, 120.0].into_iter().enumerate() {
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
            assert!(rects.iter().all(|rect| rect.is_finite() && rect.is_positive()));
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
                    egui::Pos2::ZERO, egui::vec2(width, 720.0),
                )),
                ..Default::default()
            };
            let _ = ctx.run(raw, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    row(ui, |ui| {
                        for index in 0..3 {
                            let control = numeric_control_shell(ui, 120.0, |ui| {
                                let button = ui.add_sized([120.0, TOUCH_HEIGHT], egui::Button::new("360")).rect;
                                let error = ui.colored_label(egui::Color32::RED,
                                    if index == 0 { "Please enter a valid value between 10 and 200" } else { "请输入 10 到 200" }).rect;
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
                assert!(rectangle.max.x <= width + 0.5, "width={width}: {rectangles:?}");
                for other in &rectangles[index + 1..] {
                    assert!(!rectangle.intersects(*other), "width={width}: {rectangles:?}");
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

            assert!(rects.iter().all(|rect| rect.is_finite() && rect.is_positive()));
            assert!(
                rects.iter().all(|rect| rect.max.x <= width + 0.5),
                "width={width}: {rects:?}"
            );
        }
    }
}
