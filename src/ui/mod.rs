//! UI 层：App 状态机 + 顶栏 + 标签页 + 底部状态栏。
//!
//! 后台任务与消息处理在独立 impl App 块中。
//!
//! 线程模型：所有网络/长任务 spawn thread + mpsc channel<String> 回 UI，
//! 带 200ms 轮询重绘。带内消息协议：
//! __IP_DONE__ / __LOGIN_DONE__ / __RUN_DONE__ / __AI_DONE__ / __RECORDS__ / __AI_LIST__
//! __SEMESTER__ / __CHEAT__ / __RANK__ / __USER__ / __AI_RECORDS__

pub mod ai;
pub mod data;
pub mod device;
pub mod fonts;
pub mod jobs;
pub mod msgs;
pub mod log;
pub mod records;
pub mod run;
pub mod user;
pub mod theme;

use crate::api::model::{self, Config, HeaderIdentity, Session};
use eframe::egui;
use std::sync::mpsc::{Receiver, Sender};

pub const IP: &str = "__IP_DONE__";
pub const LOGIN_DONE: &str = "__LOGIN_DONE__";
pub const RUN_DONE: &str = "__RUN_DONE__";
pub const AI_DONE: &str = "__AI_DONE__";
pub const RECORDS: &str = "__RECORDS__";
pub const AI_LIST: &str = "__AI_LIST__";
pub const AI_DETAIL: &str = "__AI_DETAIL__";
pub const SEMESTER: &str = "__SEMESTER__";
pub const CHEAT: &str = "__CHEAT__";
pub const RANK: &str = "__RANK__";
pub const USER: &str = "__USER__";
pub const AI_RECORDS: &str = "__AI_RECORDS__";
pub const RUN_DETAIL: &str = "__RUN_DETAIL__";

/// 提交结果弹窗。
pub struct PopupInfo {
    pub title: String,
    pub lines: Vec<String>,
}

/// 顶层 App。
pub struct App {
    pub tx: Sender<String>,
    pub rx: Receiver<String>,
    pub log: log::LogStore,
    pub font_loaded: Option<String>,

    pub ip: String,
    pub identity: HeaderIdentity,
    pub device_buf: HeaderIdentity,
    pub session: Option<Session>,
    pub config: Config,

    pub username: String,
    pub password: String,
    pub remember: bool,
    pub login_busy: bool,
    pub run_busy: bool,
    pub ai_busy: bool,
    pub records_busy: bool,
    pub data_busy: bool,
    pub user_busy: bool,
    pub status: String,
    pub popup: Option<PopupInfo>,
    pub ai_confirm: Option<ai::AiBatchPlan>,

    pub tab: usize,
    pub run_page: run::RunPage,
    pub ai_page: ai::AiPage,
    pub records_page: records::RecordsPage,
    pub data_page: data::DataPage,
    pub user_page: user::UserPage,
    pub device_page: device::DevicePage,
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_messages();
        ctx.request_repaint_after(std::time::Duration::from_millis(200));

        // ── 顶栏 ────────────────────────────────────────────────
        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.add_space(6.0);
            egui::Grid::new("top_row1").num_columns(6).spacing([10.0, 3.0]).show(ui, |ui| {
                ui.label("IP：");
                if self.ip.is_empty() {
                    ui.colored_label(theme::warn(), "获取中…");
                } else if self.ip == "失败" {
                    ui.colored_label(theme::err(), "失败");
                } else {
                    ui.monospace(&self.ip);
                }
                match &self.session {
                    Some(s) => {
                        ui.colored_label(theme::ok(), format!("● {}（uid={}）", s.name, s.uid));
                    }
                    None => {
                        ui.colored_label(theme::err(), "● 未登录");
                    }
                }
                ui.label("设备：");
                ui.monospace(&self.identity.device_id);
                ui.end_row();
            });
            ui.add_space(3.0);
            egui::Grid::new("top_row2").num_columns(5).spacing([8.0, 3.0]).show(ui, |ui| {
                ui.label("账号：");
                ui.add_sized(
                    [220.0, 26.0],
                    egui::TextEdit::singleline(&mut self.username),
                );
                ui.label("密码：");
                ui.add_sized(
                    [200.0, 26.0],
                    egui::TextEdit::singleline(&mut self.password).password(true),
                );
                ui.checkbox(&mut self.remember, "记住密码");
                let logged = self.session.as_ref().is_some();
                let btn_text = if logged { "登出" } else { "登录" };
                if ui.add_enabled(!self.login_busy, theme::primary_btn(btn_text)).clicked() {
                    if logged {
                        self.do_logout();
                    } else {
                        self.do_login();
                    }
                }
                ui.end_row();
            });
            ui.add_space(4.0);
        });

        // ── 底部状态栏 ──────────────────────────────────────────
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(format!("日志 {} 行", self.log.len()));
                ui.separator();
                if let Some(s) = &self.data_page.semester {
                    ui.label(format!(
                        "学期 {}：有效 {}/{} 次",
                        s.sname, s.semester_valid_count, s.semester_count
                    ));
                }
                if let Some(c) = &self.data_page.cheat {
                    if c.is_clean() {
                        ui.colored_label(theme::ok(), "自查正常");
                    } else {
                        ui.colored_label(theme::err(), format!("已标记：{}", c.self_brief()));
                    }
                }
                ui.separator();
                ui.label(&self.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // 超链接但不用链接样式，纯文本观感
                    ui.hyperlink_to(
                        egui::RichText::new("YanamiNeko").color(theme::text_dim()),
                        "https://github.com/YanamiNeko",
                    );
                    ui.label(egui::RichText::new("Powered by").color(theme::text_dim()));
                });
            });
            ui.add_space(2.0);
        });

        // ── 标签页 ──────────────────────────────────────────────
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.selectable_value(&mut self.tab, 0, "跑步");
                ui.selectable_value(&mut self.tab, 1, "AI运动");
                ui.selectable_value(&mut self.tab, 2, "运动记录");
                ui.selectable_value(&mut self.tab, 3, "数据");
                ui.selectable_value(&mut self.tab, 4, "我的");
                ui.selectable_value(&mut self.tab, 5, "设备信息");
                ui.selectable_value(&mut self.tab, 6, "运行日志");
            });
            ui.separator();
            match self.tab {
                0 => self.draw_run(ui),
                1 => self.draw_ai(ui),
                2 => self.draw_records(ui),
                3 => self.draw_data(ui),
                4 => self.draw_user(ui),
                5 => self.draw_device(ui),
                _ => self.log.render(ui),
            }
        });

        if let Some(p) = &self.popup {
            let mut open = true;
            let mut close = false;
            egui::Window::new(egui::RichText::new(&p.title).strong())
                .open(&mut open)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .order(egui::Order::Foreground)
                .show(ctx, |ui| {
                    for line in &p.lines {
                        ui.label(line);
                    }
                    ui.add_space(6.0);
                    if ui.button("关闭").clicked() {
                        close = true;
                    }
                });
            if !open || close {
                self.popup = None;
            }
        }
    }
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let font_loaded = fonts::install(&cc.egui_ctx);
        theme::apply(&cc.egui_ctx);
        let identity = model::load_identity();
        let device_buf = identity.clone();
        let session = model::load_session();
        let config = model::load_config();
        let username = config.username.clone();
        let password = config.password.clone();
        let remember = config.remember;

        let mut app = Self {
            tx: tx.clone(),
            rx,
            log: log::LogStore::default(),
            font_loaded,
            ip: String::new(),
            identity,
            device_buf,
            session: session.is_logged_in().then_some(session),
            config: config.clone(),
            username,
            password,
            remember,
            login_busy: false,
            run_busy: false,
            ai_busy: false,
            records_busy: false,
            data_busy: false,
            user_busy: false,
            status: "就绪".into(),
            popup: None,
            ai_confirm: None,
            tab: 0,
            run_page: run::RunPage {
                dist_min: config.dist_min,
                dist_max: config.dist_max,
                pace_min: config.pace_min,
                pace_max: config.pace_max,
                start_mode: 0,
                days_ago: 0,
                hour: 12,
                minute: 0,
                face_check: config.face_check,
                plan: None,
            },
            ai_page: ai::AiPage { days: 1, per_day: 1, ..Default::default() },
            records_page: records::RecordsPage::default(),
            data_page: data::DataPage::default(),
            user_page: user::UserPage::default(),
            device_page: device::DevicePage::default(),
        };
        if app.font_loaded.is_none() {
            app.log.push("未找到中文字体（msyh/simhei/simsun），界面中文可能显示为方块");
        }
        app.fetch_ip();
        // AI 项目列表先上缓存，网络刷新后覆盖
        app.ai_page.list = crate::api::model::load_ai_sports().unwrap_or_default();
        if app.session.is_some() {
            app.refresh_data_page();
            app.refresh_records();
            app.refresh_user_page();
            app.refresh_ai_list();
        }
        app
    }

    /// 后台任务（线程 + 日志 channel）。
    pub fn spawn_job<F>(&self, f: F)
    where
        F: FnOnce(Sender<String>) + Send + 'static,
    {
        let tx = self.tx.clone();
        std::thread::spawn(move || f(tx));
    }

    pub fn logger(tx: Sender<String>) -> impl FnMut(&str) {
        move |s: &str| {
            tx.send(s.to_string()).ok();
        }
    }

}
