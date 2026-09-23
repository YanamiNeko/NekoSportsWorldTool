//! 跑步页：恒 sportType=1；距离/配速范围 + 开始时间（随机或指定，最多前 3 天）。
//!
//! 开始时间：「日期」下拉（今天 / 1-3 天前）两种模式共用；随机模式在该日 7:00-20:00
//! 内抽样，并保证「开始 + 用时」不越过当前时刻；指定模式完全按用户填写的时/分
//! （尚未到达时按当前时刻），「换一版」只重掷运动量，不动时刻。

use super::map::MapState;
use super::{mobile, theme, App};
use crate::track::generate_road::RouteMode;
use chrono::{Datelike, Duration, Local, TimeZone, Timelike};
use eframe::egui;

/// 随机时刻窗口（含端点）：7:00-20:00。
const RAND_HOUR_LO: u32 = 7;
const RAND_HOUR_HI: u32 = 20;
/// flow.rs 提交时会给开始时间加 0-4s 抖动，随机模式抽样时预留，避免「开始 + 用时」越过当前时刻。
const JITTER_MARGIN_MS: i64 = 5_000;

/// 指定日期（days_ago 天前）的 7:00-20:00 内均匀抽样一个时刻，且不晚于 latest_ms。
///
/// latest_ms = 当前时刻 - 用时 - 抖动余量。若该日窗口整体晚于 latest_ms（例如凌晨选「今天」），
/// 退化为 latest_ms，宁可贴近当前时间，也不产生「未来开跑」。
fn random_time_ago(days_ago: i64, latest_ms: i64) -> i64 {
    let base = Local::now() - Duration::days(days_ago.clamp(0, 3));
    let lo = Local
        .with_ymd_and_hms(base.year(), base.month(), base.day(), RAND_HOUR_LO, 0, 0)
        .single();
    let hi = Local
        .with_ymd_and_hms(base.year(), base.month(), base.day(), RAND_HOUR_HI, 0, 0)
        .single();
    let (lo_ms, hi_ms) = match (lo, hi) {
        (Some(l), Some(h)) => (l.timestamp_millis(), h.timestamp_millis()),
        _ => return latest_ms,
    };
    let hi_ms = hi_ms.min(latest_ms);
    let lo_ms = lo_ms.min(hi_ms);
    let span = (hi_ms - lo_ms).max(0) as f64;
    lo_ms + (rand::random::<f64>() * span) as i64
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

/// 日期下拉的显示文本。
fn days_ago_label(days_ago: i64) -> String {
    match days_ago {
        0 => "今天".into(),
        1 => "昨天".into(),
        n => format!("{n} 天前"),
    }
}

#[derive(Default)]
pub struct RunPage {
    pub dist_min: f32,
    pub dist_max: f32,
    pub pace_min: f32,
    pub pace_max: f32,
    /// 手动绝对海拔（米）；空白表示使用生成器默认海拔。
    pub manual_altitude: String,
    /// 0=随机时刻 1=指定时刻
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
    /// 距离/配速是否与方案一致（不一致需重掷运动参数）。
    fn shape_matches(&self, p: &RunPlan) -> bool {
        p.dist_min == self.dist_min
            && p.dist_max == self.dist_max
            && p.pace_min == self.pace_min
            && p.pace_max == self.pace_max
    }

    /// 开始时间控件是否与方案一致。
    ///
    /// 随机模式的时刻是抽样结果（不是输入），只比对日期；指定模式的时/分是用户输入，需回比。
    fn time_matches(&self, p: &RunPlan) -> bool {
        if p.start_mode != self.start_mode || p.days_ago != self.days_ago {
            return false;
        }
        self.start_mode == 0 || (p.hour == self.hour && p.minute == self.minute)
    }

    /// 开跑时刻上限：当前时刻 - 用时 - 抖动余量（flow.rs 提交时还会再加 0-4s）。
    fn latest_start_ms(dur: i64) -> i64 {
        crate::crypto::envelope::now_ms() - dur * 1000 - JITTER_MARGIN_MS
    }

    /// 按当前参数抽样运动量（距离 / 配速 / 用时）。
    fn sample_shape(&self) -> (f64, f32, i64) {
        let (lo, hi) = (
            self.dist_min.min(self.dist_max),
            self.dist_min.max(self.dist_max),
        );
        let (plo, phi) = (
            self.pace_min.min(self.pace_max),
            self.pace_min.max(self.pace_max),
        );
        let pace = plo + (phi - plo) * rand::random::<f32>();
        let dist = (lo + (hi - lo) * rand::random::<f32>()) as f64;
        let dur = (dist * pace as f64).round() as i64;
        (dist, pace, dur)
    }

    /// 按当前模式与日期取一个开始时刻（随机模式抽样，指定模式采纳输入框）。
    fn pick_time(&self) -> i64 {
        if self.start_mode == 0 {
            random_time_ago(self.days_ago, Self::latest_start_ms(self.dur_hint()))
        } else {
            specified_time(self.days_ago, self.hour, self.minute)
        }
    }

    /// 用于随机抽样的时长上限：已抽样的方案优先，否则按当前配速区间上界估一个。
    fn dur_hint(&self) -> i64 {
        match &self.plan {
            Some(p) => p.dur,
            None => {
                let hi = self.dist_min.max(self.dist_max) as f64;
                let phi = self.pace_min.max(self.pace_max) as f64;
                (hi * phi).round() as i64
            }
        }
    }

    /// 运动参数变更时重抽距离/配速/用时，保留已选开始时刻（指定模式的时刻是用户输入，不动）。
    fn regen_shape(&mut self) {
        let (dist, pace, dur) = self.sample_shape();
        let start_ms = match (&self.plan, self.start_mode) {
            // 随机模式沿用已抽样的时刻（仅按新用时收紧上限），避免拖动距离时时刻乱跳
            (Some(p), 0) => p.start_ms.min(Self::latest_start_ms(dur)),
            _ => self.pick_time(),
        };
        self.write_plan(dist, pace, dur, start_ms);
    }

    /// 只重算开始时刻，保留已抽样的运动量（日期 / 模式变更时用）。
    fn resync_time(&mut self) {
        let start_ms = self.pick_time();
        if let Some(p) = self.plan.as_mut() {
            p.start_mode = self.start_mode;
            p.days_ago = self.days_ago;
            p.hour = self.hour;
            p.minute = self.minute;
            p.start_ms = start_ms;
        }
    }

    /// 把当前参数与已算好的量写成方案。
    fn write_plan(&mut self, dist: f64, pace: f32, dur: i64, start_ms: i64) {
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

    /// 「换一版」：重掷运动量；随机模式下同时换一个开始时刻。
    ///
    /// 指定模式的时刻由用户填写，按钮不碰它（用户填什么就是什么）。
    pub fn regen_plan(&mut self) {
        let (dist, pace, dur) = self.sample_shape();
        let start_ms = if self.start_mode == 0 {
            random_time_ago(self.days_ago, Self::latest_start_ms(dur))
        } else {
            specified_time(self.days_ago, self.hour, self.minute)
        };
        self.write_plan(dist, pace, dur, start_ms);
    }

    /// 确保方案与当前参数一致（参数变更补齐 / 提交前兜底）。
    ///
    /// 两个一致性判定都在改写方案「之前」求值：否则 regen_shape 会把当前参数写进方案，
    /// 同帧内「距离 + 日期」一起改时日期变化将被漏掉。
    pub fn ensure_plan(&mut self) {
        let shape_ok = self.plan.as_ref().map(|p| self.shape_matches(p)) == Some(true);
        let time_ok = self.plan.as_ref().map(|p| self.time_matches(p)) == Some(true);
        if !shape_ok {
            self.regen_shape();
        }
        if !time_ok {
            self.resync_time();
        }
    }

    /// 随机模式的抽样结果是否落在所选日期的 7:00-20:00 窗口内。
    ///
    /// 凌晨选「今天」时窗口尚未到达，`random_time_ago` 会退化到贴近当前时刻，
    /// 此时界面需要说明，避免用户以为刻意选了凌晨。
    fn random_window_ok(&self) -> bool {
        let Some(p) = &self.plan else { return true };
        let Some(t) = Local.timestamp_millis_opt(p.start_ms).single() else {
            return true;
        };
        let day = (Local::now() - Duration::days(self.days_ago.clamp(0, 3))).date_naive();
        t.date_naive() == day && (RAND_HOUR_LO..=RAND_HOUR_HI).contains(&t.hour())
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
            let compact = mobile::compact_ui(ui);
            let mut draw_distance_inputs = |ui: &mut egui::Ui| {
                mobile::drag_f32(
                    ui,
                    "run_dist_min",
                    &mut page.dist_min,
                    0.5..=20.0,
                    0.05,
                    2,
                    " km",
                );
                ui.label("至");
                mobile::drag_f32(
                    ui,
                    "run_dist_max",
                    &mut page.dist_max,
                    0.5..=20.0,
                    0.05,
                    2,
                    " km",
                );
            };
            if compact {
                ui.label("距离范围（km）：");
                ui.horizontal(draw_distance_inputs);
            } else {
                ui.horizontal(|ui| {
                    ui.label("距离范围（km）：");
                    draw_distance_inputs(ui);
                });
            }
            let mut draw_pace_inputs = |ui: &mut egui::Ui| {
                mobile::drag_f32(
                    ui,
                    "run_pace_min",
                    &mut page.pace_min,
                    180.0..=520.0,
                    5.0,
                    0,
                    "",
                );
                ui.label("至");
                mobile::drag_f32(
                    ui,
                    "run_pace_max",
                    &mut page.pace_max,
                    180.0..=520.0,
                    5.0,
                    0,
                    "",
                );
            };
            if compact {
                ui.label("配速范围（秒/km）：");
                ui.horizontal(draw_pace_inputs);
            } else {
                ui.horizontal(|ui| {
                    ui.label("配速范围（秒/km）：");
                    draw_pace_inputs(ui);
                });
            }
            mobile::row(ui, |ui| {
                ui.label("手动海拔（米/区间）：");
                mobile::text_edit(
                    ui,
                    "run_manual_altitude",
                    &mut page.manual_altitude,
                    crate::platform::InputKind::Text,
                    120.0,
                );
                ui.label("留空自动；可填 17.2 或 11.6-22.8");
            });
            mobile::row(ui, |ui| {
                ui.label("开始时间：");
                ui.radio_value(&mut page.start_mode, 0, "随机时刻");
                ui.radio_value(&mut page.start_mode, 1, "指定时刻");
                ui.label("日期：");
                egui::ComboBox::from_id_salt("run_days_ago")
                    .width(96.0)
                    .selected_text(days_ago_label(page.days_ago))
                    .show_ui(ui, |ui| {
                        for d in 0..=3 {
                            ui.selectable_value(&mut page.days_ago, d, days_ago_label(d));
                        }
                    });
                if page.start_mode == 0 {
                    ui.label(format!("（{RAND_HOUR_LO}:00-{RAND_HOUR_HI}:00 内随机）"));
                } else {
                    ui.label("时刻：");
                    mobile::drag_i64(ui, "run_hour", &mut page.hour, 0..=23, 1.0, "", " 点");
                    ui.label(":");
                    mobile::drag_i64(ui, "run_minute", &mut page.minute, 0..=59, 1.0, "", "");
                }
            });
            if page.start_mode == 1 && page.days_ago == 0 {
                // 今天 + 指定时刻：提示是否落在未来
                let now = Local::now();
                let spec = Local
                    .with_ymd_and_hms(
                        now.year(),
                        now.month(),
                        now.day(),
                        page.hour as u32,
                        page.minute as u32,
                        0,
                    )
                    .single();
                if let Some(t) = spec {
                    if t.timestamp_millis() > crate::crypto::envelope::now_ms() {
                        ui.colored_label(
                            theme::warn(),
                            "指定时刻在今天且尚未到达，将按当前时间提交",
                        );
                    }
                }
            }
            mobile::row(ui, |ui| {
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
        // 参数变更时补方案；显示本次提交的确定方案
        self.run_page.ensure_plan();
        self.draw_run_warnings(ui);
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
        mobile::row(ui, |ui| {
            ui.colored_label(theme::plain(), plan_label);
            if ui.small_button("换一版").clicked() {
                self.run_page.regen_plan();
            }
        });

        self.draw_run_map(ui);

        ui.add_space(8.0);
        let enabled = !self.run_busy && self.session.is_some();
        let btn = if self.run_busy {
            theme::primary_btn("提交中…")
        } else {
            theme::primary_btn("开始跑步")
        };
        mobile::row(ui, |ui| {
            if ui.add_enabled(enabled, btn).clicked() {
                self.start_run();
            }
            if self.session.is_none() {
                ui.label("（请先登录）");
            }
        });
    }

    /// 开始时间相关的提示。须在 `ensure_plan()` 之后调用，才对得上本次提交时刻。
    fn draw_run_warnings(&self, ui: &mut egui::Ui) {
        let page = &self.run_page;
        if page.start_mode == 0 {
            if !page.random_window_ok() {
                ui.colored_label(
                    theme::warn(),
                    format!(
                        "所选日期的 {RAND_HOUR_LO}:00-{RAND_HOUR_HI}:00 尚未到达，本次只能贴近当前时刻",
                    ),
                );
            }
            return;
        }
        // 今天 + 指定时刻：提示是否落在未来（与原有行为一致）
        if page.days_ago != 0 {
            return;
        }
        let now = Local::now();
        if let Some(t) = Local
            .with_ymd_and_hms(
                now.year(),
                now.month(),
                now.day(),
                page.hour.clamp(0, 23) as u32,
                page.minute.clamp(0, 59) as u32,
                0,
            )
            .single()
        {
            if t.timestamp_millis() > crate::crypto::envelope::now_ms() {
                ui.colored_label(theme::warn(), "指定时刻在今天且尚未到达，将按当前时间提交");
            }
        }
    }

    fn draw_run_map(&mut self, ui: &mut egui::Ui) {
        if self.run_page.route_mode != RouteMode::Road {
            return;
        }
        let Some(net) = self.network.clone() else {
            ui.colored_label(
                theme::warn(),
                "未加载 OSM 路网，无法预览（请先到「路网」页导入）",
            );
            return;
        };
        let Some(plan) = self.run_page.plan.clone() else {
            return;
        };

        if self.run_page.preview_stale {
            self.run_page.preview = None;
            self.run_page.map_fitted = false;
            self.run_page.preview_stale = false;
            // 点位缓存按锚点隔离（main 的串城市防护）；锚点无效时视为无缓存
            let cached = self
                .identity
                .anchor_coordinate()
                .ok()
                .and_then(crate::api::model::load_points_cache_for);
            if let Some((_ts, pts)) = cached {
                let pts_bd = crate::api::points::points_bd(&pts);
                if !pts_bd.is_empty() {
                    let fences = crate::api::model::load_fence_cache().unwrap_or_default();
                    match crate::track::generate_road::plan_road_view(
                        &net,
                        &pts_bd,
                        plan.dist * 1000.0,
                        plan.seed,
                        &fences,
                    ) {
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
        painter.rect_filled(
            canvas,
            egui::Rounding::ZERO,
            egui::Color32::from_rgb(250, 252, 251),
        );

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
                self.run_page
                    .map
                    .draw_polyline(&painter, canvas, e, road, 1.0);
            }
            let bld = egui::Color32::from_rgb(224, 194, 170);
            for b in &p.buildings {
                self.run_page
                    .map
                    .draw_polygon(&painter, canvas, b, bld, 1.0);
            }
            let fence_c = egui::Color32::from_rgb(180, 118, 0);
            for f in &p.fences {
                self.run_page
                    .map
                    .draw_polygon(&painter, canvas, f, fence_c, 2.0);
            }
            let route_c = egui::Color32::from_rgb(30, 111, 216);
            self.run_page
                .map
                .draw_polyline(&painter, canvas, &p.route, route_c, 2.5);
            let cp = egui::Color32::from_rgb(240, 180, 0);
            for &(la, lo) in &p.checkpoints {
                self.run_page
                    .map
                    .draw_point(&painter, canvas, la, lo, cp, 3.5);
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
                        plan.dist,
                        p.length_m,
                        loops,
                        p.checkpoints.len()
                    )
                } else {
                    format!(
                        "本次方案 {:.2} km · 路线 {:.0} m · {} 打卡点",
                        plan.dist,
                        p.length_m,
                        p.checkpoints.len()
                    )
                }
            } else {
                format!(
                    "本次方案 {:.2} km · {} 打卡点",
                    plan.dist,
                    p.checkpoints.len()
                )
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
        // 参数变更时补齐方案；提交直接使用预计算值
        self.run_page.ensure_plan();
        let page = &mut self.run_page;
        let plan = match page.plan.clone() {
            Some(p) => p,
            None => return,
        };
        let altitude_spec = match crate::track::altitude::parse_spec(&page.manual_altitude) {
            Ok(spec) => spec,
            Err(e) => {
                self.status = e;
                return;
            }
        };
        let (manual_altitude, manual_altitude_range) = match altitude_spec {
            None => (None, None),
            Some(crate::track::altitude::AltitudeSpec::Single(value)) => (Some(value), None),
            Some(crate::track::altitude::AltitudeSpec::Range(range)) => (None, Some(range)),
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
        self.config.manual_altitude = manual_altitude;
        self.config.manual_altitude_range = manual_altitude_range;
        self.config.route_mode = if route_mode == RouteMode::Road {
            "road".into()
        } else {
            "legacy".into()
        };
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
            let params = crate::api::flow::RunParams {
                dist,
                dur,
                start_ms,
                face_check,
                manual_altitude,
                manual_altitude_range,
                seed,
                route_mode,
            };
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

#[cfg(test)]
mod tests {
    use super::*;

    /// 一份参数合理的页面状态（配速 6:00/km，距离 2 km 附近）。
    fn page(start_mode: usize, days_ago: i64) -> RunPage {
        RunPage {
            dist_min: 2.0,
            dist_max: 2.2,
            pace_min: 350.0,
            pace_max: 370.0,
            manual_altitude: String::new(),
            start_mode,
            days_ago,
            hour: 12,
            minute: 0,
            face_check: false,
            plan: None,
            ..Default::default()
        }
    }

    /// 开跑时刻本身不得落在未来（两种模式都成立）。
    fn assert_start_not_future(p: &RunPlan) {
        let now = crate::crypto::envelope::now_ms();
        assert!(p.start_ms <= now, "开始时刻 {} 落在未来", p.start_ms);
    }

    /// 随机模式：整段跑步（含 flow.rs 的 0-4s 抖动）都不得越过当前时刻。
    ///
    /// 指定模式不适用：用户指定今天 12:00 而此刻已 12:05 时，原行为就是照常提交。
    fn assert_random_not_future(p: &RunPlan) {
        let now = crate::crypto::envelope::now_ms();
        assert!(
            p.start_ms + p.dur * 1000 + JITTER_MARGIN_MS <= now,
            "随机时刻 {} + 用时 {}s 越过当前时刻",
            p.start_ms,
            p.dur
        );
    }

    /// 指定模式下「换一版」只换运动量，用户填的时刻必须原样保留。
    #[test]
    fn shuffle_keeps_user_time_in_specified_mode() {
        let mut p = page(1, 1);
        p.hour = 9;
        p.minute = 15;
        p.ensure_plan();
        for _ in 0..40 {
            p.regen_plan();
            let plan = p.plan.clone().unwrap();
            assert_eq!((p.hour, p.minute), (9, 15), "「换一版」改动了用户填的时刻");
            assert_eq!((plan.hour, plan.minute), (9, 15), "方案时刻与输入框不一致");
        }
    }

    /// 指定模式下「换一版」仍应换掉距离/配速（否则按钮看起来没反应）。
    #[test]
    fn shuffle_changes_shape_in_specified_mode() {
        let mut p = page(1, 1);
        p.hour = 9;
        p.minute = 15;
        p.ensure_plan();
        let mut seen = std::collections::HashSet::new();
        for _ in 0..40 {
            p.regen_plan();
            seen.insert(p.plan.as_ref().unwrap().dist.to_bits());
        }
        assert!(seen.len() > 1, "「换一版」40 次仍未改变距离");
    }

    /// 日期由用户决定，「换一版」不应改动它。
    #[test]
    fn shuffle_keeps_days_ago_in_specified_mode() {
        let mut p = page(1, 2);
        for _ in 0..20 {
            p.regen_plan();
            assert_eq!(p.days_ago, 2);
            assert_eq!(p.plan.as_ref().unwrap().days_ago, 2);
        }
    }

    /// 随机模式：时刻应落在该日 7:00-20:00 内，且「换一版」每次都换新的。
    #[test]
    fn random_mode_within_window_and_varies() {
        let mut p = page(0, 1);
        let mut seen = std::collections::HashSet::new();
        for _ in 0..40 {
            p.regen_plan();
            let plan = p.plan.clone().unwrap();
            let t = Local.timestamp_millis_opt(plan.start_ms).single().unwrap();
            assert!(
                (RAND_HOUR_LO..=RAND_HOUR_HI).contains(&t.hour()),
                "随机时刻 {}:{} 越出 {RAND_HOUR_LO}:00-{RAND_HOUR_HI}:00",
                t.hour(),
                t.minute()
            );
            seen.insert((t.hour(), t.minute()));
        }
        assert!(seen.len() > 1, "随机模式 40 次仍未改变时刻");
    }

    /// 任何模式下开始时刻都不得落在未来；随机模式还要求整段跑步不越过当前时刻。
    #[test]
    fn never_schedules_future_start() {
        for mode in [0, 1] {
            for days_ago in 0..=3 {
                let mut p = page(mode, days_ago);
                for _ in 0..25 {
                    p.regen_plan();
                    assert_start_not_future(p.plan.as_ref().unwrap());
                    if mode == 0 {
                        assert_random_not_future(p.plan.as_ref().unwrap());
                    }
                }
                // 改参数走 ensure_plan 的路径同样成立
                p.dist_max = 5.0;
                p.pace_max = 500.0;
                p.ensure_plan();
                let plan = p.plan.as_ref().unwrap();
                assert_start_not_future(plan);
                if mode == 0 {
                    assert_random_not_future(plan);
                }
            }
        }
    }

    /// 今天 + 已过去的指定时刻：按原时刻提交，不动它。
    #[test]
    fn specified_today_past_hour_is_kept() {
        let mut p = page(1, 0);
        p.regen_plan();
        // 用户在界面上填时刻（regen_plan 之后填，才是用户输入而非抽样结果）
        p.hour = 0;
        p.minute = 30;
        p.ensure_plan();
        let plan = p.plan.clone().unwrap();
        let today = Local::now().date_naive();
        let want = Local
            .with_ymd_and_hms(today.year(), today.month(), today.day(), 0, 30, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        // 凌晨 0:30 尚未到达时会被钳制，跳过断言
        if want <= crate::crypto::envelope::now_ms() {
            assert_eq!(plan.start_ms, want, "已过去的指定时刻不应被改动");
        }
        assert_start_not_future(&plan);
    }

    /// 未来的指定时刻被钳制到当前时刻；已过去的时刻原样保留。
    ///
    /// 断言直接对齐契约 `min(填入时刻, 现在)`，与运行时刻无关（23:59 跑也不会误报）。
    #[test]
    fn specified_time_is_clamped_to_now() {
        let now = Local::now();
        let mut p = page(1, 0);
        p.hour = 23;
        p.minute = 59;
        p.regen_plan();
        let plan = p.plan.clone().unwrap();

        let today = now.date_naive();
        let want = Local
            .with_ymd_and_hms(today.year(), today.month(), today.day(), 23, 59, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        let now_ms = crate::crypto::envelope::now_ms();
        assert_eq!(
            plan.start_ms,
            want.min(now_ms),
            "指定时刻未按 min(填入, 现在) 处理"
        );
        assert_start_not_future(&plan);
    }

    /// 改距离/配速只重抽运动量，不应打乱用户指定的时刻。
    #[test]
    fn shape_change_keeps_specified_time() {
        let mut p = page(1, 1);
        p.regen_plan();
        // 用户在界面上填时刻
        p.hour = 9;
        p.minute = 15;
        p.ensure_plan();
        let before = p.plan.clone().unwrap();
        assert_eq!((before.hour, before.minute), (9, 15));

        p.dist_min = 3.0;
        p.dist_max = 3.5;
        p.ensure_plan();
        let after = p.plan.clone().unwrap();
        assert_ne!(
            (after.dist, after.pace),
            (before.dist, before.pace),
            "运动量未重抽"
        );
        assert_eq!(after.start_ms, before.start_ms, "改距离不应改动指定时刻");
        assert_eq!((p.hour, p.minute), (9, 15));
    }

    /// 改小时/分钟应立刻反映到方案，且不重抽运动量。
    #[test]
    fn editing_hhmm_resyncs_plan() {
        let mut p = page(1, 1);
        p.regen_plan();
        let before = p.plan.clone().unwrap();

        p.hour = 8;
        p.minute = 45;
        p.ensure_plan();
        let after = p.plan.clone().unwrap();
        assert_eq!((after.hour, after.minute), (8, 45));
        assert_eq!(
            (after.dist, after.pace, after.dur),
            (before.dist, before.pace, before.dur)
        );
        assert_start_not_future(&after);
    }

    /// 切换模式应立即换掉时刻，且不重抽运动量。
    #[test]
    fn mode_switch_resyncs_time_only() {
        let mut p = page(1, 1);
        p.regen_plan();
        let before = p.plan.clone().unwrap();

        p.start_mode = 0;
        p.ensure_plan();
        let after = p.plan.clone().unwrap();
        assert_eq!(after.start_mode, 0);
        assert_eq!(
            (after.dist, after.pace, after.dur),
            (before.dist, before.pace, before.dur)
        );
        assert_random_not_future(&after);
    }

    /// 日期下拉作用于两种模式。
    #[test]
    fn date_change_applies_to_both_modes() {
        for mode in [0, 1] {
            let mut p = page(mode, 0);
            p.regen_plan();
            p.days_ago = 3;
            p.ensure_plan();
            let plan = p.plan.clone().unwrap();
            assert_eq!(plan.days_ago, 3);
            let want = (Local::now() - Duration::days(3)).date_naive();
            let t = Local.timestamp_millis_opt(plan.start_ms).single().unwrap();
            assert_eq!(t.date_naive(), want, "mode={mode} 未落到 3 天前");
        }
    }

    /// 日期下拉文本。
    #[test]
    fn days_ago_labels() {
        assert_eq!(days_ago_label(0), "今天");
        assert_eq!(days_ago_label(1), "昨天");
        assert_eq!(days_ago_label(3), "3 天前");
    }

    /// 同一帧内「距离 + 日期」一起改：两处变化都不能漏。
    #[test]
    fn simultaneous_shape_and_date_change() {
        let mut p = page(0, 0);
        p.regen_plan();
        p.dist_min = 4.0;
        p.dist_max = 4.5;
        p.days_ago = 2;
        p.ensure_plan();
        let plan = p.plan.clone().unwrap();
        assert_eq!(plan.days_ago, 2, "日期变化被漏掉");
        assert!(plan.dist >= 4.0, "距离变化被漏掉");
        let want = (Local::now() - Duration::days(2)).date_naive();
        let t = Local.timestamp_millis_opt(plan.start_ms).single().unwrap();
        assert_eq!(t.date_naive(), want);
        assert_random_not_future(&plan);
    }

    /// 同一帧内「时刻 + 距离」一起改。
    #[test]
    fn simultaneous_shape_and_hhmm_change() {
        let mut p = page(1, 1);
        p.regen_plan();
        p.dist_min = 4.0;
        p.dist_max = 4.5;
        p.hour = 8;
        p.minute = 20;
        p.ensure_plan();
        let plan = p.plan.clone().unwrap();
        assert_eq!((plan.hour, plan.minute), (8, 20), "时刻变化被漏掉");
        assert!(plan.dist >= 4.0, "距离变化被漏掉");
        assert_eq!(plan.start_ms, specified_time(1, 8, 20));
        assert_start_not_future(&plan);
    }

    /// 今天 + 指定时刻且该时刻已过：照常提交，不再回退（用户指定的时刻说了算）。
    #[test]
    fn specified_past_time_is_committed_as_is() {
        let now = Local::now();
        // 取一个今天已过去足够久的整点，保证整段用时都已结束
        let h = now.hour().saturating_sub(3) as i64;
        let mut p = page(1, 0);
        p.hour = h;
        p.minute = 0;
        p.regen_plan();
        let plan = p.plan.clone().unwrap();
        let today = now.date_naive();
        let want = Local
            .with_ymd_and_hms(today.year(), today.month(), today.day(), h as u32, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        assert_eq!(plan.start_ms, want, "用户填的过去时刻被改动了");
        assert_start_not_future(&plan);
    }

    /// 明天（未来）的指定时刻必然被钳制，且界面可据此提示。
    ///
    /// 用「未来某天」构造，避免依赖当前钟点（23:xx 跑也不会失效）。
    #[test]
    fn future_specified_time_is_clamped_and_detectable() {
        // days_ago 取负即未来，但会被 clamp 到 0；这里直接构造今天最后一刻
        let mut p = page(1, 0);
        p.hour = 23;
        p.minute = 59;
        p.ensure_plan();
        let plan = p.plan.clone().unwrap();

        let now = Local::now();
        let today = now.date_naive();
        let typed = Local
            .with_ymd_and_hms(today.year(), today.month(), today.day(), 23, 59, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        let now_ms = crate::crypto::envelope::now_ms();
        assert_eq!(plan.start_ms, typed.min(now_ms));
        assert_start_not_future(&plan);
        // 界面提示条件：填入时刻晚于现在（23:59 总成立，除非正好在那一分钟）
        if typed > now_ms {
            assert!(plan.start_ms < typed, "未来时刻未被钳制");
        }
    }

    /// 反复调用 ensure_plan 应当是幂等的（不改动任何已定值）。
    #[test]
    fn ensure_plan_is_idempotent() {
        for mode in [0, 1] {
            let mut p = page(mode, 1);
            p.regen_plan();
            let first = p.plan.clone().unwrap();
            for _ in 0..5 {
                p.ensure_plan();
            }
            let again = p.plan.clone().unwrap();
            assert_eq!(
                (again.dist, again.pace, again.dur, again.start_ms),
                (first.dist, first.pace, first.dur, first.start_ms),
                "mode={mode} ensure_plan 不稳定"
            );
        }
    }

    /// 随机模式抽样结果应落在所选日期的窗口内；凌晨选「今天」时窗口未到，
    /// 此时允许退化，但必须能被 random_window_ok 识别出来（界面据此提示）。
    #[test]
    fn random_window_detection() {
        let mut p = page(0, 1);
        p.regen_plan();
        assert!(p.random_window_ok(), "昨天的随机时刻应当落在窗口内");

        // 构造一个「今天但早于 7:00」的方案，窗口判定应为 false
        let now = Local::now();
        let early = Local
            .with_ymd_and_hms(now.year(), now.month(), now.day(), 3, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        p.days_ago = 0;
        if let Some(plan) = p.plan.as_mut() {
            plan.days_ago = 0;
            plan.start_ms = early;
        }
        assert!(
            !p.random_window_ok(),
            "今天 03:00 不应被判为在 7:00-20:00 窗口内"
        );
    }
}
