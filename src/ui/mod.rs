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
pub mod components;
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
pub mod run;
pub mod theme;
pub mod user;

use crate::api::model::{self, Config, HeaderIdentity, Session};
use eframe::egui;
use std::sync::mpsc::{Receiver, Sender};
use std::sync::Arc;

/// 长字符串（设备号等）截断为前 N 字符 + 省略号。
fn short_id(id: &str) -> String {
    let mut s: String = id.chars().take(8).collect();
    if id.chars().count() > 8 {
        s.push('…');
    }
    s
}

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
    pub login_open: bool,
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
        self.draw_overlays(ctx);
        crate::platform::sync_clipboard(ctx);
    }
}

impl App {
    /// 悬浮层：登录弹窗、提示弹窗、更新窗口。
    ///
    /// 与 `draw_shell` 分开，既方便单测按需驱动，也让骨架与悬浮层的绘制顺序显式可见。
    fn draw_overlays(&mut self, ctx: &egui::Context) {
        self.draw_login_window(ctx);

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
    }

    /// 桌面侧栏品牌区。
    fn draw_side_brand(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            egui::Frame::none()
                .fill(theme::ACCENT_500)
                .rounding(egui::Rounding::same(theme::radius::SM))
                .inner_margin(egui::Margin::same(theme::space::SM))
                .show(ui, |ui| {
                    ui.label(
                        egui::RichText::new("N")
                            .color(egui::Color32::WHITE)
                            .strong(),
                    );
                });
            ui.vertical(|ui| {
                ui.label(
                    egui::RichText::new("NekoSports")
                        .size(theme::FONT_BODY)
                        .strong()
                        .color(theme::ACCENT_700),
                );
                ui.label(
                    egui::RichText::new(format!("WorldTool · v{}", crate::platform::version_name()))
                        .size(theme::FONT_SMALL)
                        .color(theme::TEXT_FAINT),
                );
            });
        });
    }

    /// 顶栏：登录态 / IP / 设备号三枚状态胶囊 + 登录（登出）按钮。
    /// 手机宽拆成两行（胶囊一行、按钮一行），避免窄屏下按钮把胶囊挤成竖排。
    fn draw_top_bar(&mut self, ui: &mut egui::Ui, compact: bool) {
        ui.add_space(theme::space::SM);
        let logged = self.session.is_some();
        let name_text = match &self.session {
            Some(s) => Some(format!("● {}（uid={}）", s.name, s.uid)),
            None => None,
        };
        let ip_text = if self.ip.is_empty() {
            None
        } else if self.ip == "失败" {
            Some("IP 失败".to_string())
        } else {
            Some(format!("IP {}", self.ip))
        };
        let device_text = format!("设备 {}", short_id(&self.identity.device_id));
        let busy = self.login_busy;

        let chip_identity = |ui: &mut egui::Ui| match &name_text {
            Some(n) => components::chip(ui, n, components::Tone::Ok),
            None => components::chip(ui, "● 未登录", components::Tone::Err),
        };
        let chip_ip = |ui: &mut egui::Ui| match &ip_text {
            Some(t) => {
                let tone = if t.starts_with("IP 失败") {
                    components::Tone::Err
                } else {
                    components::Tone::Neutral
                };
                components::chip(ui, t, tone);
            }
            None => components::chip(ui, "IP 获取中", components::Tone::Warn),
        };
        let login_btn = |ui: &mut egui::Ui, clicked: &mut bool| {
            let btn = if logged {
                theme::secondary_btn("登出")
            } else if self.auth_expired {
                theme::primary_btn("重新登录")
            } else {
                theme::primary_btn("登录")
            };
            if ui.add_enabled(!busy, btn).clicked() {
                *clicked = true;
            }
        };

        let mut login_clicked = false;
        if compact {
            // 手机宽：第一行登录态 + IP，第二行设备号 + 登录按钮。
            // 必须包在 horizontal 里，否则 with_layout 会吃掉整个面板的剩余高度。
            ui.horizontal(|ui| {
                chip_identity(ui);
                chip_ip(ui);
            });
            ui.add_space(theme::space::XS);
            ui.horizontal(|ui| {
                components::chip(ui, &device_text, components::Tone::Neutral);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    login_btn(ui, &mut login_clicked);
                });
            });
        } else {
            ui.horizontal_wrapped(|ui| {
                chip_identity(ui);
                chip_ip(ui);
                components::chip(ui, &device_text, components::Tone::Neutral);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    login_btn(ui, &mut login_clicked);
                });
            });
        }
        ui.add_space(theme::space::SM);
        if login_clicked {
            if logged {
                self.do_logout();
            } else {
                // 账号或密码为空时先给出校验提示，再打开弹窗引导填写；
                // 已保存凭据则直接进弹窗确认后提交。
                if self.username.trim().is_empty() || self.password.is_empty() {
                    self.status = "请填写账号和密码".into();
                }
                self.login_open = true;
            }
        }
    }

    /// 桌面底部状态栏。
    fn draw_status_bar(&mut self, ui: &mut egui::Ui) {
        ui.add_space(theme::space::XS);
        let log_len = self.log.len();
        let status = self.status.clone();
        let semester_text = self.data_page.semester.as_ref().map(|s| {
            format!(
                "学期 {}：有效 {}/{} 次",
                s.sname, s.semester_valid_count, s.semester_count
            )
        });
        let cheat_text = self.data_page.cheat.as_ref().map(|c| {
            let tone = match c.status() {
                crate::api::cheat::Status::NoRecord => components::Tone::Ok,
                crate::api::cheat::Status::Flagged => components::Tone::Err,
                _ => components::Tone::Warn,
            };
            (tone, c.label().to_string())
        });
        let updating = self.update.checking || self.update.downloading;
        let version = crate::platform::version_name();
        let mut check_clicked = false;

        ui.horizontal_wrapped(|ui| {
            components::chip(ui, &format!("日志 {} 行", log_len), components::Tone::Neutral);
            if let Some(s) = &semester_text {
                ui.label(egui::RichText::new(s).size(theme::FONT_SMALL).color(theme::TEXT_DIM));
            }
            if let Some((tone, text)) = &cheat_text {
                components::chip(ui, text, *tone);
            }
            ui.separator();
            ui.label(egui::RichText::new(&status).size(theme::FONT_SMALL).color(theme::TEXT_DIM));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(
                        !updating,
                        egui::Button::new(
                            egui::RichText::new("检查更新").small().color(theme::text_dim()),
                        ),
                    )
                    .clicked()
                {
                    check_clicked = true;
                }
                ui.label(
                    egui::RichText::new(format!("v{}", version))
                        .small()
                        .color(theme::text_dim()),
                );
            });
        });
        ui.add_space(theme::space::XS);
        if check_clicked {
            self.check_update(true);
        }
    }

    /// 登录弹窗。
    fn draw_login_window(&mut self, ctx: &egui::Context) {
        if !self.login_open {
            return;
        }
        let mut open = true;
        let busy = self.login_busy;
        let mut submit = false;
        let mut cancel = false;
        egui::Window::new(egui::RichText::new("登录").strong())
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_width(280.0);
                egui::Grid::new("login_grid")
                    .num_columns(2)
                    .spacing([8.0, 8.0])
                    .show(ui, |ui| {
                        ui.label("账号");
                        mobile::text_edit(
                            ui,
                            "login_username",
                            &mut self.username,
                            crate::platform::InputKind::Text,
                            180.0,
                        );
                        ui.end_row();
                        ui.label("密码");
                        mobile::text_edit(
                            ui,
                            "login_password",
                            &mut self.password,
                            crate::platform::InputKind::Password,
                            180.0,
                        );
                        ui.end_row();
                    });
                ui.checkbox(&mut self.remember, "记住密码");
                ui.add_space(theme::space::SM);
                ui.horizontal(|ui| {
                    if ui
                        .add_enabled(!busy, theme::primary_btn("登录"))
                        .clicked()
                    {
                        submit = true;
                    }
                    if ui.add(theme::ghost_btn("取消")).clicked() {
                        cancel = true;
                    }
                });
            });
        if !open || cancel {
            self.login_open = false;
        }
        if submit {
            self.login_open = false;
            self.do_login();
        }
    }

    /// 应用骨架：桌面侧栏 / 顶栏 / 手机底部导航 / 内容区。
    fn draw_shell(&mut self, ctx: &egui::Context) {
        // ── 桌面分组侧栏 ──────────────────────────────────────
        let compact = mobile::is_compact(ctx.screen_rect().width());
        if !compact {
            egui::SidePanel::left("nav_panel")
                .default_width(200.0)
                .resizable(false)
                .show(ctx, |ui| {
                    ui.add_space(theme::space::SM);
                    self.draw_side_brand(ui);
                    ui.separator();
                    mobile::side_nav(ui, &mut self.tab);
                });
        }

        // ── 顶栏（状态胶囊 + 登录）────────────────────────────
        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            self.draw_top_bar(ui, compact);
        });

        // ── 底部：手机导航 / 桌面状态栏 ───────────────────────
        if compact {
            egui::TopBottomPanel::bottom("nav_bar").show(ctx, |ui| {
                mobile::bottom_nav(ui, &mut self.tab);
            });
        } else {
            egui::TopBottomPanel::bottom("status_bar").show(ctx, |ui| {
                self.draw_status_bar(ui);
            });
        }

        // ── 内容区 ────────────────────────────────────────────
        egui::CentralPanel::default().show(ctx, |ui| {
            // 手机宽：工具页（设备信息/运行日志/路网/关于）是「我的」的二级页，
            // 顶部给一条返回条；桌面宽由左侧分组侧栏直达，无需返回条。
            if compact && self.tab >= 5 {
                let title = mobile::page_label(self.tab);
                if mobile::sub_page_header(ui, title) {
                    self.tab = mobile::MOBILE_HOME;
                }
            }
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
            login_open: false,
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
        if app.session.is_some() {
            app.refresh_data_page();
            app.refresh_records();
            app.refresh_user_page();
            app.refresh_ai_list();
            if app.config.osm_path.is_empty() {
                app.refresh_points();
            }
        }
        // 已配置 OSM 路网则启动时后台加载
        if !app.config.osm_path.is_empty() {
            let path = app.config.osm_path.clone();
            app.load_osm_async(path);
        }
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
