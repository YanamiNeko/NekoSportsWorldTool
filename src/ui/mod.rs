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
pub mod records;
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

/// 按字符数截断文本。
#[allow(dead_code)]
fn truncate(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        text.to_string()
    } else {
        let mut s: String = text.chars().take(max_chars).collect();
        s.push('…');
        s
    }
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
pub const FENCE_DONE: &str = "__FENCE_DONE__";
pub const POINTS_DONE: &str = "__POINTS_DONE__";

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
    pub login_open: bool,
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
    pub update: about::UpdateUi,

    /// 已加载的 OSM 路网（真实道路路由 + 地图显示共用）。
    pub network: Option<Arc<route_planner::RoadGraph>>,
    pub osm_page: osm::OsmPage,
    net_tx: Sender<Result<Arc<route_planner::RoadGraph>, String>>,
    net_rx: Receiver<Result<Arc<route_planner::RoadGraph>, String>>,
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
        crate::platform::sync_clipboard(ctx);
    }
}

impl App {
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
    fn draw_top_bar(&mut self, ui: &mut egui::Ui, _compact: bool) {
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

        let mut login_clicked = false;
        ui.horizontal_wrapped(|ui| {
            match &name_text {
                Some(n) => components::chip(ui, n, components::Tone::Ok),
                None => components::chip(ui, "● 未登录", components::Tone::Err),
            }
            match &ip_text {
                Some(t) => {
                    let tone = if t.starts_with("IP 失败") {
                        components::Tone::Err
                    } else {
                        components::Tone::Neutral
                    };
                    components::chip(ui, t, tone);
                }
                None => components::chip(ui, "IP 获取中", components::Tone::Warn),
            }
            components::chip(ui, &device_text, components::Tone::Neutral);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let btn = if logged {
                    theme::secondary_btn("登出")
                } else {
                    theme::primary_btn("登录")
                };
                if ui.add_enabled(!busy, btn).clicked() {
                    login_clicked = true;
                }
            });
        });
        ui.add_space(theme::space::SM);
        if login_clicked {
            if logged {
                self.do_logout();
            } else {
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
            if c.is_clean() {
                (components::Tone::Ok, "自查正常".to_string())
            } else {
                (components::Tone::Err, format!("已标记：{}", c.self_brief()))
            }
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

    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        let (tx, rx) = std::sync::mpsc::channel();
        let (net_tx, net_rx) = std::sync::mpsc::channel();
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
            login_open: false,
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
                map_fitted: false,
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
        };
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
            app.refresh_fence();
            app.refresh_points();
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
