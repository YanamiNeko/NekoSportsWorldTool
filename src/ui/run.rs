//! 跑步页：恒 sportType=1；距离/配速范围 + 开始时间（随机或指定，最多前 3 天）。

use super::{theme, App};
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
        });
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
                ui.add(egui::DragValue::new(&mut page.pace_min).range(180..=900).speed(5));
                ui.label("至");
                ui.add(egui::DragValue::new(&mut page.pace_max).range(180..=900).speed(5));
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
        self.config.dist_min = page.dist_min;
        self.config.dist_max = page.dist_max;
        self.config.pace_min = page.pace_min;
        self.config.pace_max = page.pace_max;
        self.config.face_check = page.face_check;
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
        self.spawn_job(move |tx| {
            let mut log = App::logger(tx.clone());
            let seed = (crate::crypto::envelope::now_ms() % 2_147_483_647) as u64;
            let mut client = crate::api::client::ApiClient::new(identity, Some(session));
            let params = crate::api::flow::RunParams { dist, dur, start_ms, face_check, seed };
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
