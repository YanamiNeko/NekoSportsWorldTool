//! Compact open-source and free-distribution notice for the status bar.

use eframe::egui;

const NOTICE: &str =
    "本项目免费提供。请遵守项目许可证，勿将本软件付费转售；如通过付费渠道获得，请联系售卖者。";
const REPOSITORY: &str = "https://github.com/YanamiNeko/NekoSportsWorldTool";

/// Keeps the notice in the existing status bar so it does not reduce page height.
pub fn show(ui: &mut egui::Ui) -> egui::Response {
    ui.hyperlink_to("开源", REPOSITORY).on_hover_text(NOTICE)
}
