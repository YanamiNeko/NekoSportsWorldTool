//! 关于页与自动更新流程：检查 / 下载进度 / 确认弹窗 / 重启或安装收尾。
//!
//! 桌面端替换 exe 后走 relaunch 重启；Android 端下载 APK 到私有目录后
//! 交给系统安装器（platform::install_apk）。

use super::{theme, App};
use crate::update::ReleaseInfo;
use eframe::egui;

/// 更新流程的 UI 状态（随 App 存活，消息协议回填）。
#[derive(Default)]
pub struct UpdateUi {
    pub checking: bool,
    pub latest: Option<ReleaseInfo>,
    pub up_to_date: bool,
    pub check_error: Option<String>,
    pub downloading: bool,
    pub done: u64,
    pub total: Option<u64>,
    /// 等待用户确认开始下载（启动检查发现新版本时也置此）。
    pub confirm: Option<ReleaseInfo>,
    /// 下载替换完成后的收尾动作。
    pub finish: Option<FinishAction>,
    /// 启动询问模式：是否检查更新。
    pub ask_startup: bool,
}

#[derive(Clone)]
pub enum FinishAction {
    /// 桌面：替换完成，等待重启。
    Restart { tag: String },
    /// Android：APK 已就绪，等待调起安装器（桌面构建下不会构造）。
    #[cfg_attr(not(target_os = "android"), allow(dead_code))]
    InstallApk { tag: String },
}

impl App {
    pub fn draw_about(&mut self, ui: &mut egui::Ui) {
        ui.heading("NekoSportsWorldTool");
        ui.add_space(6.0);
        ui.label(format!("当前版本：v{}", crate::platform::version_name()));
        let repo = if cfg!(target_os = "android") {
            crate::update::REPO_FORK
        } else {
            crate::update::REPO_UPSTREAM
        };
        ui.hyperlink_to(
            egui::RichText::new(format!("https://github.com/{repo}/releases")).color(theme::text_dim()),
            format!("https://github.com/{repo}/releases"),
        );
        ui.add_space(4.0);
        ui.colored_label(
            theme::text_dim(),
            "手动更新：下载对应平台的新程序文件；Windows 用户退出软件后，用新的 nekosportsworldtool.exe 替换旧文件。",
        );
        ui.colored_label(
            theme::text_dim(),
            "请保留 config.json、identity.json、points_cache.json 等配置文件，不要删除或修改。",
        );
        ui.add_space(10.0);

        // ── 检查 / 版本信息 ────────────────────────────────────
        ui.horizontal(|ui| {
            let busy = self.update.checking || self.update.downloading;
            if ui.add_enabled(!busy, theme::primary_btn("检查更新")).clicked() {
                self.check_update(true);
            }
            if self.update.checking {
                ui.colored_label(theme::text_dim(), "检查中…");
            } else if self.update.up_to_date {
                ui.colored_label(theme::ok(), "√ 已是最新版本");
            }
        });
        if let Some(e) = &self.update.check_error {
            ui.colored_label(theme::err(), format!("× {e}"));
        }
        if let Some(rel) = &self.update.latest {
            ui.add_space(6.0);
            ui.label(format!(
                "最新版本：{}（{:.1} MB，{}）",
                rel.tag,
                rel.asset_size as f64 / 1024.0 / 1024.0,
                rel.repo
            ));
            if !rel.title.is_empty() {
                ui.colored_label(theme::text_dim(), &rel.title);
            }
            let notes: Vec<&str> = rel.notes.lines().take(12).collect();
            if !notes.is_empty() {
                ui.add_space(4.0);
                egui::ScrollArea::vertical()
                    .id_salt("about_notes")
                    .max_height(160.0)
                    .show(ui, |ui| {
                        for line in notes {
                            ui.label(egui::RichText::new(line).small().color(theme::text_dim()));
                        }
                    });
            }
            ui.add_space(6.0);
            if self.update.downloading {
                self.draw_download_progress(ui);
            } else if self.update.finish.is_none() {
                if ui.button("立即更新").clicked() {
                    if self.any_busy() {
                        self.status = "有任务进行中，请稍后再试".into();
                    } else {
                        let rel = rel.clone();
                        self.update.confirm = Some(rel);
                    }
                }
            }
        }
        if self.update.downloading && self.update.latest.is_none() {
            self.draw_download_progress(ui);
        }
        if let Some(fin) = self.update.finish.clone() {
            ui.add_space(6.0);
            match fin {
                FinishAction::Restart { tag } => {
                    ui.colored_label(theme::ok(), format!("√ 已更新到 {tag}，重启后生效"));
                }
                FinishAction::InstallApk { tag } => {
                    ui.colored_label(theme::ok(), format!("√ 已下载 {tag}，请在弹窗中安装"));
                }
            }
        }

        ui.add_space(14.0);
        ui.separator();

        // ── 启动检查模式 ───────────────────────────────────────
        ui.label("启动时检查更新：");
        ui.horizontal(|ui| {
            for (m, label) in
                [("silent", "静默检查（发现新版才提示）"), ("ask", "每次询问"), ("off", "关闭")]
            {
                let selected = self.config.update_check == m;
                if ui.radio(selected, label).clicked() {
                    self.config.update_check = m.to_string();
                    if crate::api::model::save_config(&self.config).is_ok() {
                        self.status = "已保存启动检查设置".into();
                    }
                }
            }
        });
        ui.colored_label(
            theme::text_dim(),
            "协议变更时旧版本可能异常，建议保持开启",
        );
    }

    fn draw_download_progress(&self, ui: &mut egui::Ui) {
        let bar = egui::ProgressBar::new(self.update.fraction())
            .text(self.update.progress_text());
        ui.add(bar);
    }

    /// 全局窗口：启动询问 / 确认更新 / 更新收尾（不依赖当前标签页）。
    pub fn draw_update_windows(&mut self, ctx: &egui::Context) {
        if self.update.ask_startup {
            let mut done: Option<&str> = None;
            egui::Window::new(egui::RichText::new("检查更新").strong())
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    ui.set_max_width(320.0);
                    ui.label("是否检查新版本？\n协议变更时旧版本可能异常，建议检查。");
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.add(theme::primary_btn("检查")).clicked() {
                            done = Some("check");
                        }
                        if ui.button("本次跳过").clicked() {
                            done = Some("skip");
                        }
                        if ui.button("不再询问").clicked() {
                            done = Some("off");
                        }
                    });
                });
            if let Some(choice) = done {
                self.update.ask_startup = false;
                match choice {
                    "check" => self.check_update(false),
                    "off" => {
                        self.config.update_check = "off".into();
                        let _ = crate::api::model::save_config(&self.config);
                    }
                    _ => {}
                }
            }
        }

        if let Some(rel) = self.update.confirm.clone() {
            let mut close = false;
            let mut go = false;
            egui::Window::new(egui::RichText::new(format!("发现新版本 {}", rel.tag)).strong())
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    ui.set_max_width((ctx.screen_rect().width() - 32.0).min(460.0));
                    if !rel.title.is_empty() {
                        ui.label(&rel.title);
                    }
                    ui.label(format!(
                        "大小 {:.1} MB · 来源 {}",
                        rel.asset_size as f64 / 1024.0 / 1024.0,
                        rel.repo
                    ));
                    let notes: Vec<&str> = rel.notes.lines().take(8).collect();
                    if !notes.is_empty() {
                        ui.add_space(4.0);
                        for line in notes {
                            ui.label(egui::RichText::new(line).small());
                        }
                    }
                    ui.add_space(4.0);
                    ui.colored_label(theme::warn(), "更新会替换为官方 Release 版本并重启程序");
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.add(theme::primary_btn("立即更新")).clicked() {
                            go = true;
                        }
                        if ui.button("忽略").clicked() {
                            close = true;
                        }
                    });
                });
            if go || close {
                self.update.confirm = None;
            }
            if go {
                if self.any_busy() {
                    self.status = "有任务进行中，请稍后再试".into();
                } else {
                    self.start_update_download(rel);
                }
            }
        }

        if let Some(fin) = self.update.finish.clone() {
            let mut done = false;
            let (title, body, button) = match fin {
                FinishAction::Restart { tag } => (
                    format!("已更新到 {tag}"),
                    "程序已替换完成，重启后使用新版本。".to_string(),
                    "重启程序".to_string(),
                ),
                FinishAction::InstallApk { tag } => (
                    format!("已下载 {tag}"),
                    "点击安装，在系统弹窗中确认；若提示禁止安装，请允许本应用安装未知应用。".to_string(),
                    "安装".to_string(),
                ),
            };
            egui::Window::new(egui::RichText::new(&title).strong())
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    ui.set_max_width(360.0);
                    ui.label(&body);
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui.add(theme::primary_btn(&button)).clicked() {
                            done = true;
                        }
                        if ui.button("稍后").clicked() {
                            self.update.finish = None;
                        }
                    });
                });
            if done {
                match self.update.finish.take() {
                    Some(FinishAction::Restart { .. }) => {
                        match crate::relaunch::relaunch_self() {
                            Ok(()) => {
                                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                            }
                            Err(e) => {
                                self.status = e.clone();
                                self.popup = Some(super::PopupInfo {
                                    title: "更新完成".into(),
                                    lines: vec![e],
                                });
                            }
                        }
                    }
                    Some(FinishAction::InstallApk { .. }) => {
                        crate::platform::install_apk(
                            &crate::platform::data_dir()
                                .join(crate::update::APK_NAME)
                                .to_string_lossy(),
                        );
                        self.update.finish = None;
                    }
                    None => {}
                }
            }
        }
    }
}

impl UpdateUi {
    fn fraction(&self) -> f32 {
        match self.total {
            Some(t) if t > 0 => (self.done as f32 / t as f32).clamp(0.0, 1.0),
            _ => 0.0,
        }
    }

    fn progress_text(&self) -> String {
        let mb = |n: u64| format!("{:.1} MB", n as f64 / 1024.0 / 1024.0);
        match self.total {
            Some(t) if t > 0 => format!("{}/{}", mb(self.done), mb(t)),
            _ => format!("已下载 {}", mb(self.done)),
        }
    }
}
