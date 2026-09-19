//! 跑步页：恒 sportType=1；距离/配速范围 + 开始时间（随机或指定，最多前 3 天）。

use super::map::MapState;
use super::{theme, App};
use crate::track::generate_road::RouteMode;
use chrono::{Datelike, Duration, Local, TimeZone};
use eframe::egui;

/// days_ago 天前的随机运动时刻（7:00-20:00）。
#[allow(dead_code)]
fn day_time_ago(days_ago: i64) -> i64 {
    let base = Local::now() - Duration::days(days_ago);
    let h = 7 + rand::random::<u32>() % 14;
    let m = rand::random::<u32>() % 60;
    Local
        .with_ymd_and_hms(base.year(), base.month(), base.day(), h, m, rand::random::<u32>() % 60)
        .single()
        .map(|t| t.timestamp_millis())
        .unwrap_or_else(crate::crypto::envelope::now_ms)
}

/// 指定时刻：days_ago(0=今天) + 时/分；超出 3 天或在未来时做钳制。
fn specified_time(days_ago: i64, hour: i64, minute: i64) -> i64 {
    let now = Local::now();
    let base = now - Duration::days(days_ago.clamp(0, 3));
    let h = hour.clamp(0, 23);
    let m = minute.clamp(0, 59);
    let t = Local
        .with_ymd_and_hms(base.year(), base.month(), base.day(), h as u32, m as u32, 0)
        .single()
        .map(|x| x.timestamp_millis())
        .unwrap_or_else(crate::crypto::envelope::now_ms);
    t.min(now.timestamp_millis())
}

#[derive(Default)]
pub struct RunPage {
    pub dist_min: f32,
    pub dist_max: f32,
    pub pace_min: f32,
    pub pace_max: f32,
    /// 0=随机过去时间 1=指定时间
    pub start_mode: usize,
    pub days_ago: i64,
    pub hour: i64,
    pub minute: i64,
    pub face_check: bool,
    /// 预计算方案：参数变更时重抽样，提交直接使用
    pub plan: Option<RunPlan>,
    /// 路线算法模式
    pub route_mode: RouteMode,
    /// 地图视图状态
    pub map: MapState,
    /// 路网预览（真实道路路由）
    pub preview: Option<crate::track::generate_road::RoadPlan>,
    /// 预览是否过期（参数变更后置真）
    pub preview_stale: bool,
    /// 地图视野是否已适配
    pub map_fitted: bool,
}

/// 一次提交的确定方案（进入页面/参数变更时抽样生成）。
#[derive(Debug, Clone)]
pub struct RunPlan {
    pub dist_min: f32,
    pub dist_max: f32,
    pub pace_min: f32,
    pub pace_max: f32,
    pub start_mode: usize,
    pub days_ago: i64,
    pub hour: i64,
    pub minute: i64,
    /// 公里
    pub dist: f64,
    /// 秒/km
    pub pace: f32,
    /// 秒
    pub dur: i64,
    pub start_ms: i64,
    /// 本次方案的随机种子（预览与提交共用，保证所见即所得）
    pub seed: u64,
}

impl RunPage {
    fn matches(&self, p: &RunPlan) -> bool {
        p.dist_min == self.dist_min
            && p.dist_max == self.dist_max
            && p.pace_min == self.pace_min
            && p.pace_max == self.pace_max
            && p.start_mode == self.start_mode
            && (self.start_mode == 0 || (p.days_ago == self.days_ago && p.hour == self.hour && p.minute == self.minute))
    }

    /// 参数变更或手动刷新时抽样一份确定方案。
    pub fn regen_plan(&mut self) {
        let (lo, hi) = (self.dist_min.min(self.dist_max), self.dist_min.max(self.dist_max));
        let (plo, phi) = (self.pace_min.min(self.pace_max), self.pace_min.max(self.pace_max));
        let r = rand::random::<f32>();
        let pace = plo + (phi - plo) * r;
        let dist = (lo + (hi - lo) * rand::random::<f32>()) as f64;
        let dur = (dist * pace as f64).round() as i64;
        let start_ms = match self.start_mode {
            0 => crate::crypto::envelope::now_ms()
                - 30 * 60_000
                - (rand::random::<f64>() * 270.0 * 60_000.0) as i64,
            _ => specified_time(self.days_ago, self.hour, self.minute),
        };
        self.plan = Some(RunPlan {
            dist_min: self.dist_min,
            dist_max: self.dist_max,
            pace_min: self.pace_min,
            pace_max: self.pace_max,
            start_mode: self.start_mode,
            days_ago: self.days_ago,
            hour: self.hour,
            minute: self.minute,
            dist,
            pace,
            dur,
            start_ms,
            seed: rand::random::<u64>(),
        });
        self.preview_stale = true;
    }
}

impl App {
    pub fn draw_run(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                self.draw_run_content(ui);
            });
    }

    fn draw_run_content(&mut self, ui: &mut egui::Ui) {
        {
            let page = &mut self.run_page;
            ui.horizontal(|ui| {
                ui.label("距离范围（km）：");
                ui.add(egui::DragValue::new(&mut page.dist_min).range(0.5..=20.0).speed(0.05).suffix(" km"));
                ui.label("至");
                ui.add(egui::DragValue::new(&mut page.dist_max).range(0.5..=20.0).speed(0.05).suffix(" km"));
            });
            ui.horizontal(|ui| {
                ui.label("配速范围（秒/km）：");
                ui.add(egui::DragValue::new(&mut page.pace_min).range(180..=520).speed(5));
                ui.label("至");
                ui.add(egui::DragValue::new(&mut page.pace_max).range(180..=520).speed(5));
            });
            ui.horizontal(|ui| {
                ui.label("开始时间：");
                ui.radio_value(&mut page.start_mode, 0, "随机过去时间");
                ui.radio_value(&mut page.start_mode, 1, "指定时间");
                if page.start_mode == 0 {
                    ui.label("（30-300 分钟前随机）");
                } else {
                    ui.label("几天前：");
                    ui.add(egui::DragValue::new(&mut page.days_ago).range(0..=3).suffix(" 天"));
                    ui.label("时刻：");
                    ui.add(egui::DragValue::new(&mut page.hour).range(0..=23).suffix(" 点"));
                    ui.label(":");
                    ui.add(egui::DragValue::new(&mut page.minute).range(0..=59).prefix(":"));
                }
            });
            if page.start_mode == 1 && page.days_ago == 0 {
                // 今天 + 指定时刻：提示是否落在未来
                let now = Local::now();
                let spec = Local
                    .with_ymd_and_hms(now.year(), now.month(), now.day(), page.hour as u32, page.minute as u32, 0)
                    .single();
                if let Some(t) = spec {
                    if t.timestamp_millis() > crate::crypto::envelope::now_ms() {
                        ui.colored_label(theme::warn(), "指定时刻在今天且尚未到达，将按当前时间提交");
                    }
                }
            }
            ui.horizontal(|ui| {
                ui.label("人脸校验标记：");
                ui.checkbox(&mut page.face_check, "faceCheck=1");
            });
            ui.horizontal(|ui| {
                ui.label("路线算法：");
                ui.selectable_value(&mut page.route_mode, RouteMode::Legacy, "经典打卡点环");
                ui.selectable_value(&mut page.route_mode, RouteMode::Road, "真实道路路由");
                if page.route_mode == RouteMode::Road && self.network.is_none() {
                    ui.colored_label(theme::warn(), "（需先在「路网」页导入 OSM）");
                }
            });
        }

        ui.add_space(8.0);
        let fmt_pace = |s: f32| format!("{}:{:02}", (s / 60.0) as i64, (s as i64) % 60);
        let fmt_dur = |s: i64| {
            if s >= 3600 {
                format!("{}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60)
            } else {
                format!("{}:{:02}", s / 60, s % 60)
            }
        };
        // 参数变更时重抽样；显示本次提交的确定方案
        if self.run_page.plan.as_ref().map(|p| self.run_page.matches(p)) != Some(true) {
            self.run_page.regen_plan();
        }
        let plan_label = match &self.run_page.plan {
            Some(p) => {
                let start = chrono::Local
                    .timestamp_millis_opt(p.start_ms)
                    .single()
                    .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
                    .unwrap_or_default();
                let (dist, pace, dur) = (p.dist, p.pace, p.dur);
                format!(
                    "本次方案：距离 {dist:.2} km · 配速 {}/km · 用时 {} · 开始 {start}",
                    fmt_pace(pace),
                    fmt_dur(dur)
                )
            }
            None => String::new(),
        };
        ui.horizontal(|ui| {
            ui.colored_label(theme::plain(), plan_label);
            if ui.small_button("换一版").clicked() {
                self.run_page.regen_plan();
            }
        });

        self.draw_run_map(ui);

        ui.add_space(8.0);
        let enabled = !self.run_busy && self.session.is_some();
        let btn = if self.run_busy { theme::primary_btn("提交中…") } else { theme::primary_btn("开始跑步") };
        ui.horizontal(|ui| {
            if ui.add_enabled(enabled, btn).clicked() {
                self.start_run();
            }
            if self.session.is_none() {
                ui.label("（请先登录）");
            }
        });
    }

    fn draw_run_map(&mut self, ui: &mut egui::Ui) {
        if self.run_page.route_mode != RouteMode::Road {
            return;
        }
        let Some(net) = self.network.clone() else {
            ui.colored_label(theme::warn(), "未加载 OSM 路网，无法预览（请先到「路网」页导入）");
            return;
        };
        let Some(plan) = self.run_page.plan.clone() else { return };

        if self.run_page.preview_stale {
            self.run_page.preview = None;
            self.run_page.map_fitted = false;
            self.run_page.preview_stale = false;
            if let Some((_ts, pts)) = crate::api::model::load_points_cache() {
                let pts_bd = crate::api::points::points_bd(&pts);
                if !pts_bd.is_empty() {
                    let fences = crate::api::model::load_fence_cache().unwrap_or_default();
                    match crate::track::generate_road::plan_road_view(&net, &pts_bd, plan.dist * 1000.0, plan.seed, &fences) {
                        Ok(p) => self.run_page.preview = Some(p),
                        Err(e) => self.status = format!("路线预览失败：{e}"),
                    }
                } else {
                    self.status = "无打卡点缓存，提交后可回显轨迹".into();
                }
            } else {
                self.status = "无打卡点缓存，提交后可回显轨迹".into();
            }
        }

        if self.run_page.preview.is_some() {
            super::map::legend(
                ui,
                &[
                    ("道路", egui::Color32::from_rgb(200, 208, 204)),
                    ("建筑", egui::Color32::from_rgb(224, 194, 170)),
                    ("路线", egui::Color32::from_rgb(30, 111, 216)),
                    ("打卡点", egui::Color32::from_rgb(240, 180, 0)),
                    ("起点", egui::Color32::from_rgb(22, 160, 90)),
                    ("终点", egui::Color32::from_rgb(220, 38, 38)),
                ],
            );
        }

        ui.add_space(4.0);
        let rect = ui.available_rect_before_wrap();
        let (response, painter) = ui.allocate_painter(
            egui::Vec2::new(rect.width().max(200.0), 240.0),
            egui::Sense::drag(),
        );
        let canvas = response.rect;
        painter.rect_filled(canvas, egui::Rounding::ZERO, egui::Color32::from_rgb(250, 252, 251));

        // 首次预览后适配视野：优先以电子围栏为中点/范围，无围栏时退化为路线+打卡点+道路
        if !self.run_page.map_fitted {
            let mut bounds: Vec<(f64, f64)> = Vec::new();
            if let Some(p) = &self.run_page.preview {
                for f in &p.fences {
                    bounds.extend(f.iter().copied());
                }
                if bounds.is_empty() {
                    bounds = super::map::collect_bounds(&p.edges);
                    bounds.extend(p.route.iter().copied());
                    bounds.extend(p.checkpoints.iter().copied());
                }
            }
            if !bounds.is_empty() {
                self.run_page.map.fit(canvas, &bounds);
                self.run_page.map_fitted = true;
            }
        }

        if let Some(p) = &self.run_page.preview {
            let road = egui::Color32::from_rgb(200, 208, 204);
            for e in &p.edges {
                self.run_page.map.draw_polyline(&painter, canvas, e, road, 1.0);
            }
            let bld = egui::Color32::from_rgb(224, 194, 170);
            for b in &p.buildings {
                self.run_page.map.draw_polygon(&painter, canvas, b, bld, 1.0);
            }
            let fence_c = egui::Color32::from_rgb(180, 118, 0);
            for f in &p.fences {
                self.run_page.map.draw_polygon(&painter, canvas, f, fence_c, 2.0);
            }
            let route_c = egui::Color32::from_rgb(30, 111, 216);
            self.run_page.map.draw_polyline(&painter, canvas, &p.route, route_c, 2.5);
            let cp = egui::Color32::from_rgb(240, 180, 0);
            for &(la, lo) in &p.checkpoints {
                self.run_page.map.draw_point(&painter, canvas, la, lo, cp, 3.5);
            }
            if let Some(first) = p.route.first() {
                self.run_page.map.draw_point(
                    &painter,
                    canvas,
                    first.0,
                    first.1,
                    egui::Color32::from_rgb(22, 160, 90),
                    4.5,
                );
            }
            if let Some(last) = p.route.last() {
                self.run_page.map.draw_point(
                    &painter,
                    canvas,
                    last.0,
                    last.1,
                    egui::Color32::from_rgb(220, 38, 38),
                    4.5,
                );
            }
            let label = if p.length_m > 0.0 {
                let loops = plan.dist * 1000.0 / p.length_m;
                if loops > 1.15 {
                    format!(
                        "本次方案 {:.2} km · 单圈 {:.0} m × {:.1} 圈 · {} 打卡点",
                        plan.dist, p.length_m, loops, p.checkpoints.len()
                    )
                } else {
                    format!("本次方案 {:.2} km · 路线 {:.0} m · {} 打卡点", plan.dist, p.length_m, p.checkpoints.len())
                }
            } else {
                format!("本次方案 {:.2} km · {} 打卡点", plan.dist, p.checkpoints.len())
            };
            painter.text(
                egui::Pos2::new(canvas.left() + 8.0, canvas.top() + 8.0),
                egui::Align2::LEFT_TOP,
                label,
                egui::FontId::proportional(12.0),
                egui::Color32::from_rgb(88, 104, 99),
            );
        } else {
            painter.text(
                canvas.center(),
                egui::Align2::CENTER_CENTER,
                "（无预览）",
                egui::FontId::proportional(14.0),
                egui::Color32::from_rgb(150, 160, 156),
            );
        }

        self.run_page.map.interact(ui, canvas);
    }

    fn start_run(&mut self) {
        let page = &mut self.run_page;
        // 参数变更时确保方案最新；提交直接使用预计算值
        if page.plan.as_ref().map(|p| page.matches(p)) != Some(true) {
            page.regen_plan();
        }
        let plan = match page.plan.clone() {
            Some(p) => p,
            None => return,
        };
        let (dist, dur) = (plan.dist * 1000.0, plan.dur); // 米
        let start_ms = plan.start_ms;
        let face_check = if page.face_check { 1 } else { 0 };
        let route_mode = page.route_mode;
        self.config.dist_min = page.dist_min;
        self.config.dist_max = page.dist_max;
        self.config.pace_min = page.pace_min;
        self.config.pace_max = page.pace_max;
        self.config.face_check = page.face_check;
        self.config.route_mode = if route_mode == RouteMode::Road { "road".into() } else { "legacy".into() };
        let _ = crate::api::model::save_config(&self.config);

        let identity = self.identity.clone();
        let session = match self.session.clone() {
            Some(s) => s,
            None => {
                self.status = "请先登录".into();
                return;
            }
        };
        self.run_busy = true;
        self.status = "跑步提交中…".into();
        let seed = plan.seed;
        self.spawn_job(move |tx| {
            let mut log = App::logger(tx.clone());
            let mut client = crate::api::client::ApiClient::new(identity, Some(session));
            let params = crate::api::flow::RunParams { dist, dur, start_ms, face_check, seed, route_mode };
            let payload = match crate::api::flow::run_full_flow(&mut client, &params, &mut log) {
                Ok(out) => {
                    log(&format!(
                        "全链完成 rrid={} obs={}/2 verify={} uuid={}",
                        out.result.rrid, out.obs_ok, out.detail_ok, out.result.uuid
                    ));
                    serde_json::json!({
                        "ok": true, "rrid": out.result.rrid,
                        "obs_ok": out.obs_ok, "verify": out.detail_ok,
                        "uuid": out.result.uuid,
                        "dist": out.result.total_dis, "dur": out.result.total_time,
                        "steps": out.result.total_steps, "avg_step_freq": out.result.avg_step_freq,
                        "calorie": out.result.calorie, "avg_power": out.result.avg_power,
                        "sel_distance": out.result.sel_distance, "start": out.result.start_ms,
                    })
                }
                Err(e) => {
                    log(&format!("跑步提交失败: {e}"));
                    serde_json::json!({ "ok": false, "message": e })
                }
            };
            tx.send(format!("__RUN_DONE__{payload}")).ok();
        });
    }
}
