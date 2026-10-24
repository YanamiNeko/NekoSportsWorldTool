//! UI 层：App 状态机 + 顶栏 + 标签页 + 底部状态栏。
//!
//! 后台任务与消息处理在独立 impl App 块中。
//!
//! 线程模型：所有网络/长任务 spawn thread + mpsc channel<String> 回 UI，
//! 带 200ms 轮询重绘。带内消息协议：
//! __IP_DONE__ / __LOGIN_DONE__ / __RUN_DONE__ / __AI_DONE__ / __RECORDS__ / __AI_LIST__
//! __SEMESTER__ / __CHEAT__ / __RANK__ / __USER__ / __AI_RECORDS__

pub mod about;
pub mod ai;
pub mod data;
pub mod device;
pub mod fonts;
pub mod jobs;
pub mod log;
pub mod map;
pub mod mobile;
pub mod msgs;
pub mod osm;
mod preview_jobs;
pub mod records;
mod road_tools;
mod campus_lookup;
pub mod run;
pub mod theme;
pub mod user;

use crate::api::model::{self, Config, HeaderIdentity, Session};
use eframe::egui;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

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
pub const UPDATE_CHK: &str = "__UPDATE_CHK__";
pub const UPDATE_PROG: &str = "__UPDATE_PROG__";
pub const UPDATE_DONE: &str = "__UPDATE_DONE__";
pub const POINTS_DONE: &str = "__POINTS_DONE__";

/// 提交结果弹窗。
pub struct PopupInfo {
    pub title: String,
    pub lines: Vec<String>,
}

/// 顶层 App。
#[derive(Clone)]
pub struct JobSender {
    tx: Sender<String>,
    auth: Option<crate::api::client::AuthContext>,
}
impl JobSender {
    pub fn send(&self, message: String) -> Result<(), std::sync::mpsc::SendError<String>> {
        self.tx.send(match &self.auth {
            Some(context) => format!("__SESSION_JOB__{}|{message}", context.epoch),
            None => message,
        })
    }
    pub fn client(
        &self,
        identity: HeaderIdentity,
        session: Option<Session>,
    ) -> crate::api::client::ApiClient {
        crate::api::client::ApiClient::new(identity, session).with_auth_context(self.auth.clone())
    }
}

pub struct App {
    #[cfg(test)]
    route_fixture: Option<(String, std::path::PathBuf)>,
    pub session_epoch: u64,
    pub auth_guard: Arc<std::sync::atomic::AtomicBool>,
    auth_tx: Sender<crate::api::client::AuthFailure>,
    auth_rx: Receiver<crate::api::client::AuthFailure>,
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
    pub auth_expired: bool,
    pub run_busy: bool,
    pub ai_busy: bool,
    pub ai_submit_active: bool,
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
    pub update: about::UpdateUi,

    /// 已加载的 OSM 路网（真实道路路由 + 地图显示共用）。
    pub network: Option<Arc<route_planner::RoadGraph>>,
    pub osm_page: osm::OsmPage,
    net_tx: Sender<osm::NetworkLoad>,
    net_rx: Receiver<osm::NetworkLoad>,
    preview_tx: Sender<preview_jobs::PreviewResult>,
    preview_rx: Receiver<preview_jobs::PreviewResult>,
    run_control: Arc<crate::task_control::Control>,
}

impl eframe::App for App {
    #[cfg(target_os = "android")]
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        visuals.panel_fill.to_normalized_gamma_f32()
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, input: &mut egui::RawInput) {
        crate::platform::apply_safe_area(ctx, input);
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.poll_messages();
        #[cfg(target_os = "android")]
        self.poll_device_info();
        crate::platform::set_keep_screen_on(self.run_busy || self.ai_busy || self.login_busy);
        ctx.request_repaint_after(std::time::Duration::from_millis(200));

        self.draw_shell(ctx);

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
                    ui.set_max_width((ctx.screen_rect().width() - 32.0).min(520.0));
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
        self.draw_update_windows(ctx);
        crate::platform::sync_clipboard(ctx);
    }
}

impl App {
    fn draw_shell(&mut self, ctx: &egui::Context) {
        // ── 顶栏 ────────────────────────────────────────────────
        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.add_space(6.0);
            let compact = mobile::compact_ui(ui);
            let logged = self.session.is_some();
            let btn_text = if logged {
                "登出"
            } else if self.auth_expired {
                "重新登录"
            } else {
                "登录"
            };
            let mut login_clicked = false;
            let draw_connection = |ui: &mut egui::Ui| {
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
            };
            if compact {
                ui.horizontal_wrapped(|ui| {
                    ui.label(if logged { "已登录" } else { "未登录" });
                    if let Some(session) = &self.session {
                        ui.label(&session.name);
                    }
                });
                egui::CollapsingHeader::new("账号与登录")
                    .open(self.auth_expired.then_some(true))
                    .default_open(false)
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("compact_account")
                            .max_height((ctx.screen_rect().height() * 0.3).min(200.0))
                            .show(ui, |ui| {
                                ui.label(format!("设备：{}", self.identity.device_id));
                                ui.add_space(4.0);
                                ui.label("账号：");
                                mobile::text_edit(
                                    ui,
                                    "login_username",
                                    &mut self.username,
                                    crate::platform::InputKind::Text,
                                    ui.available_width(),
                                );
                                ui.label("密码：");
                                mobile::text_edit(
                                    ui,
                                    "login_password",
                                    &mut self.password,
                                    crate::platform::InputKind::Password,
                                    ui.available_width(),
                                );
                                ui.horizontal_wrapped(|ui| {
                                    ui.checkbox(&mut self.remember, "记住密码");
                                    login_clicked = ui
                                        .add_enabled(!self.login_busy, theme::primary_btn(btn_text))
                                        .clicked();
                                });
                            });
                    });
            } else {
                egui::Grid::new("top_row1")
                    .num_columns(6)
                    .spacing([10.0, 3.0])
                    .show(ui, |ui| {
                        draw_connection(ui);
                        ui.label("设备：");
                        ui.monospace(&self.identity.device_id);
                        ui.end_row();
                    });
                ui.add_space(3.0);
                egui::Grid::new("top_row2")
                    .num_columns(5)
                    .spacing([8.0, 3.0])
                    .show(ui, |ui| {
                        ui.label("账号：");
                        mobile::text_edit(
                            ui,
                            "login_username",
                            &mut self.username,
                            crate::platform::InputKind::Text,
                            220.0,
                        );
                        ui.label("密码：");
                        mobile::text_edit(
                            ui,
                            "login_password",
                            &mut self.password,
                            crate::platform::InputKind::Password,
                            200.0,
                        );
                        ui.checkbox(&mut self.remember, "记住密码");
                        login_clicked = ui
                            .add_enabled(!self.login_busy, theme::primary_btn(btn_text))
                            .clicked();
                        ui.end_row();
                    });
            }
            if login_clicked {
                if logged {
                    self.do_logout();
                } else {
                    self.do_login();
                }
            }
            ui.add_space(4.0);
        });

        // ── 底部状态栏 ──────────────────────────────────────────
        egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
            if mobile::compact_ui(ui) {
                ui.horizontal(|ui| {
                    ui.add(egui::Label::new(&self.status).truncate());
                });
                return;
            }
            ui.horizontal_wrapped(|ui| {
                ui.label(format!("日志 {} 行", self.log.len()));
                ui.separator();
                if let Some(s) = &self.data_page.semester {
                    ui.label(s.display());
                }
                if let Some(c) = &self.data_page.cheat {
                    ctx.request_repaint_after(std::time::Duration::from_secs(1));
                    ui.label(c.label());
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
                    ui.separator();
                    if ui
                        .add_enabled(
                            !(self.update.checking || self.update.downloading),
                            egui::Button::new(
                                egui::RichText::new("检查更新")
                                    .small()
                                    .color(theme::text_dim()),
                            ),
                        )
                        .clicked()
                    {
                        self.check_update(true);
                    }
                    ui.label(
                        egui::RichText::new(format!("v{}", crate::platform::build_version()))
                            .small()
                            .color(theme::text_dim()),
                    );
                });
            });
            ui.add_space(2.0);
        });

        // ── 标签页 ──────────────────────────────────────────────
        egui::CentralPanel::default().show(ctx, |ui| {
            mobile::tab_bar(ui, &mut self.tab);
            ui.separator();
            match self.tab {
                0 => self.draw_run(ui),
                1 => self.draw_ai(ui),
                2 => self.draw_records(ui),
                3 => self.draw_data(ui),
                4 => self.draw_user(ui),
                5 => self.draw_device(ui),
                6 => self.log.render(ui),
                7 => self.draw_osm_page(ui),
                _ => self.draw_about(ui),
            }
        });
    }
}

impl App {
    fn from_state(
        ctx: &egui::Context,
        identity: HeaderIdentity,
        session: Session,
        config: Config,
    ) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let (net_tx, net_rx) = std::sync::mpsc::channel();
        let (preview_tx, preview_rx) = std::sync::mpsc::channel();
        let font_loaded = fonts::install(ctx);
        theme::apply(ctx);
        let device_buf = identity.clone();
        let username = config.username.clone();
        let password = config.password.clone();
        let remember = config.remember;

        let (auth_tx, auth_rx) = std::sync::mpsc::channel();
        Self {
            #[cfg(test)]
            route_fixture: None,
            session_epoch: 0,
            auth_guard: Arc::new(std::sync::atomic::AtomicBool::new(false)),
            auth_tx,
            auth_rx,
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
            auth_expired: false,
            run_busy: false,
            ai_busy: false,
            ai_submit_active: false,
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
                manual_altitude: config
                    .manual_altitude_range
                    .map(|r| format!("{}-{}", r.min_m, r.max_m))
                    .or_else(|| config.manual_altitude.map(|v| v.to_string()))
                    .unwrap_or_default(),
                start_mode: 0,
                days_ago: 0,
                hour: 12,
                minute: 0,
                face_check: config.face_check,
                plan: None,
                route_mode: crate::track::generate_road::RouteMode::from_str(&config.route_mode),
                map: map::MapState::default(),
                preview: None,
                preview_stale: true,
                preview_inputs: None,
                map_fitted: false,
                ..Default::default()
            },
            ai_page: ai::AiPage {
                days: 1,
                per_day: 1,
                ..Default::default()
            },
            records_page: records::RecordsPage::default(),
            data_page: data::DataPage::default(),
            user_page: user::UserPage::default(),
            device_page: device::DevicePage::default(),
            update: about::UpdateUi::default(),
            network: None,
            osm_page: osm::OsmPage {
                path: config.osm_path.clone(),
                ..Default::default()
            },
            net_tx,
            net_rx,
            preview_tx,
            preview_rx,
            run_control: Arc::new(crate::task_control::Control::default()),
        }
    }

    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let mut app = Self::from_state(
            &cc.egui_ctx,
            model::load_identity(),
            model::load_session(),
            model::load_config(),
        );
        if app.font_loaded.is_none() {
            app.log
                .push("未找到中文字体（msyh/simhei/simsun），界面中文可能显示为方块");
        }
        // 上次更新残留的 .old/.new 顺手清掉
        crate::update::cleanup_residue();
        // 启动检查更新（silent 静默 / ask 询问 / off 关闭）
        match app.config.update_check.as_str() {
            "off" => {}
            "ask" => app.update.ask_startup = true,
            _ => app.check_update(false),
        }
        app.fetch_ip();
        #[cfg(target_os = "android")]
        crate::android::request_device_info(true);
        // AI 项目列表先上缓存，网络刷新后覆盖
        app.ai_page.list = crate::api::model::load_ai_sports().unwrap_or_default();
        if app.session.is_some() && app.config.offline_preview.is_none() {
            app.refresh_data_page();
            app.refresh_records();
            app.refresh_user_page();
            app.refresh_ai_list();
        }
        app.restore_route_on_startup();
        app
    }

    /// 后台任务（线程 + 日志 channel）。
    pub fn spawn_job<F>(&self, f: F)
    where
        F: FnOnce(JobSender) + Send + 'static,
    {
        let tx = JobSender {
            tx: self.tx.clone(),
            auth: None,
        };
        std::thread::spawn(move || f(tx));
    }

    pub fn spawn_session_job<F>(&self, f: F)
    where
        F: FnOnce(JobSender) + Send + 'static,
    {
        let tx = JobSender {
            tx: self.tx.clone(),
            auth: Some(crate::api::client::AuthContext {
                epoch: self.session_epoch,
                rejected: self.auth_guard.clone(),
                notify: self.auth_tx.clone(),
            }),
        };
        std::thread::spawn(move || f(tx));
    }

    fn renew_session_jobs(&mut self) {
        self.data_page.cheat = None;
        self.data_page.cheat_error = None;
        self.data_page.cheat_checking = false;
        self.session_epoch = self.session_epoch.wrapping_add(1);
        self.auth_guard = Arc::new(std::sync::atomic::AtomicBool::new(false));
    }

    pub fn logger(tx: JobSender) -> impl FnMut(&str) {
        move |s: &str| {
            tx.send(s.to_string()).ok();
        }
    }
}

#[cfg(test)]
mod layout_tests;
