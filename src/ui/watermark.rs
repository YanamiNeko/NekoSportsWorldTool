//! 主界面状态栏上方的开源信息水印。

use eframe::egui;

use super::theme;

const REPOSITORY_URL: &str = "https://github.com/YanamiNeko/NekoSportsWorldTool";
const CONTRIBUTORS_URL: &str =
    "https://github.com/YanamiNeko/NekoSportsWorldTool/graphs/contributors";

/// 在专用底部面板中绘制水印。面板由 egui 正常参与布局，
/// 因此不会覆盖中央页面的按钮或地图控件。
pub fn show(ctx: &egui::Context) -> egui::Rect {
    egui::TopBottomPanel::bottom("watermark_bar")
        .resizable(false)
        .min_height(70.0)
        .max_height(100.0)
        .show(ctx, draw)
        .response
        .rect
}

fn draw(ui: &mut egui::Ui) {
    egui::Frame::none()
        .fill(egui::Color32::from_rgba_unmultiplied(235, 241, 238, 218))
        .stroke(egui::Stroke::new(
            1.0_f32,
            egui::Color32::from_rgba_unmultiplied(13, 148, 136, 110),
        ))
        .rounding(egui::Rounding::same(6.0))
        .inner_margin(egui::Margin::same(7.0))
        .show(ui, |ui| {
            ui.set_max_width(320.0_f32.min(ui.available_width()));
            ui.label(
                egui::RichText::new("本项目免费提供，禁止违反许可证规定进行商业使用。")
                    .small()
                    .strong()
                    .color(theme::text_dim()),
            );
            ui.add(
                egui::Label::new(
                    egui::RichText::new("如果您通过付费渠道获得本软件，请联系售卖者。")
                        .small()
                        .color(theme::warn()),
                )
                .wrap(),
            );
            ui.horizontal_wrapped(|ui| {
                ui.hyperlink_to(
                    egui::RichText::new("GitHub 项目主页")
                        .small()
                        .color(theme::text_dim()),
                    REPOSITORY_URL,
                );
                ui.label(egui::RichText::new("|").small().color(theme::text_dim()));
                ui.hyperlink_to(
                    egui::RichText::new("查看贡献者")
                        .small()
                        .color(theme::text_dim()),
                    CONTRIBUTORS_URL,
                );
            });
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn watermark_reserves_space_above_status_and_below_content() {
        for width in [320.0, 390.0, 1280.0] {
            let ctx = egui::Context::default();
            // egui resolves initial panel heights over successive frames.
            for _ in 0..3 {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::Pos2::ZERO,
                        egui::vec2(width, 600.0),
                    )),
                    ..Default::default()
                };
                let _ = ctx.run(input, |ctx| {
                    let status = egui::TopBottomPanel::bottom("status_bar")
                        .show(ctx, |ui| {
                            ui.label("Status");
                        })
                        .response
                        .rect;
                    let watermark = show(ctx);
                    let content = egui::CentralPanel::default()
                        .show(ctx, |ui| {
                            let bottom = ui.max_rect().right_bottom();
                            ui.put(
                                egui::Rect::from_min_size(
                                    bottom - egui::vec2(100.0, 20.0),
                                    egui::vec2(100.0, 20.0),
                                ),
                                egui::Button::new("Action"),
                            );
                        })
                        .response
                        .rect;
                    assert!(
                        watermark.bottom() <= status.top(),
                        "watermark={watermark:?} status={status:?} content={content:?}"
                    );
                    assert!(content.bottom() <= watermark.top());
                    assert!(watermark.left() >= 0.0 && watermark.right() <= width);
                });
            }
        }
    }
}
