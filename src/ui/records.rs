//! 运动记录页：跑步 / AI 两个分栏，egui_extras 表格。

use super::{theme, App};
use chrono::TimeZone;
use eframe::egui;
use egui_extras::{Column, TableBuilder};

#[derive(Default)]
pub struct RecordsPage {
    pub rows: Vec<crate::api::records::RecordRow>,
    pub ai_groups: Vec<crate::api::ai::AiRecordGroup>,
    pub ai_total: i64,
    pub sub: usize,
    /// 当前展开详情的 rrid
    pub detail_rrid: Option<i64>,
    pub detail_raw: Option<serde_json::Value>,
    pub detail_loading: bool,
}

impl App {
    pub fn draw_records(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            ui.selectable_value(&mut self.records_page.sub, 0, "跑步");
            ui.selectable_value(&mut self.records_page.sub, 1, "AI 运动");
            ui.add_space(8.0);
            match self.records_page.sub {
                0 => {
                    if ui
                        .add_enabled(!self.records_busy, theme::primary_btn("刷新"))
                        .clicked()
                    {
                        self.refresh_records();
                    }
                    if self.records_busy {
                        ui.label("拉取中…");
                    }
                }
                _ => {
                    if ui
                        .add_enabled(!self.records_busy, theme::primary_btn("刷新全部记录"))
                        .clicked()
                    {
                        self.refresh_ai_records();
                    }
                    if self.records_busy {
                        ui.label("拉取中…");
                    }
                }
            }
        });
        ui.separator();

        match self.records_page.sub {
            0 => self.draw_run_view(ui),
            _ => self.draw_ai_table(ui),
        }
    }

    /// 跑步视图：列表与详情两个页面互斥。
    fn draw_run_view(&mut self, ui: &mut egui::Ui) {
        if self.records_page.detail_rrid.is_some() {
            self.draw_detail_view(ui);
        } else {
            self.draw_run_table(ui);
        }
    }

    /// 详情独立页：返回 + 全部字段。
    fn draw_detail_view(&mut self, ui: &mut egui::Ui) {
        let rrid = self.records_page.detail_rrid.unwrap_or(0);
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if ui.button("← 返回列表").clicked() {
                self.records_page.detail_rrid = None;
                self.records_page.detail_raw = None;
                self.records_page.detail_loading = false;
            }
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(format!("跑步详情 #{}", rrid))
                    .strong()
                    .color(theme::accent()),
            );
        });
        ui.separator();
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if self.records_page.detail_loading && self.records_page.detail_raw.is_none() {
                    ui.label("加载详情中…");
                } else if let Some(raw) = &self.records_page.detail_raw {
                    draw_detail_panel(ui, raw);
                } else {
                    ui.label("（无详情数据）");
                }
            });
    }

    fn draw_run_table(&mut self, ui: &mut egui::Ui) {
        if self.records_page.rows.is_empty() {
            ui.add_space(20.0);
            ui.centered_and_justified(|ui| ui.label("暂无跑步记录"));
            return;
        }
        let fmt_time = |ms: i64| -> String {
            chrono::Local
                .timestamp_millis_opt(ms)
                .single()
                .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
                .unwrap_or_default()
        };
        let fmt_dur = |s: i64| -> String {
            if s >= 3600 {
                format!("{}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60)
            } else {
                format!("{}:{:02}", s / 60, s % 60)
            }
        };
        let pace_of = |dis: f64, t: i64| -> (i64, i64) {
            let p = if dis > 0.0 { t as f64 / (dis / 1000.0) } else { 0.0 };
            ((p / 60.0) as i64, (p as i64) % 60)
        };

        let toggle_target = std::cell::Cell::new(None::<i64>);

        TableBuilder::new(ui)
            .striped(true)
            .vscroll(true)
            .auto_shrink([false, false])
            .column(Column::auto().at_least(130.0)) // 时间
            .column(Column::auto().at_least(70.0)) // 距离
            .column(Column::auto().at_least(70.0)) // 时长
            .column(Column::auto().at_least(60.0)) // 配速
            .column(Column::auto().at_least(70.0)) // 步频
            .column(Column::auto().at_least(60.0)) // 达标
            .column(Column::auto().at_least(90.0)) // rrid
            .column(Column::remainder()) // 操作
            .header(22.0, |mut h| {
                for t in ["时间", "距离", "时长", "配速", "步频", "达标", "rrid", "操作"] {
                    h.col(|ui| {
                        ui.label(egui::RichText::new(t).strong().color(theme::text()));
                    });
                }
            })
            .body(|mut body| {
                let sel = self.records_page.detail_rrid;
                for r in &self.records_page.rows {
                    let rrid = r.rrid;
                    let is_detail = sel == Some(rrid);
                    body.row(20.0, |mut row| {
                        row.col(|ui| {
                            let color = if is_detail { theme::accent() } else { theme::text() };
                            ui.label(egui::RichText::new(fmt_time(r.start_time)).color(color));
                        });
                        row.col(|ui| {
                            ui.monospace(format!("{:.2} km", r.total_dis / 1000.0));
                        });
                        row.col(|ui| {
                            ui.monospace(fmt_dur(r.total_time));
                        });
                        row.col(|ui| {
                            let (m, s) = pace_of(r.total_dis, r.total_time);
                            ui.monospace(format!("{m}:{s:02}"));
                        });
                        row.col(|ui| {
                            ui.monospace(format!("{} spm", r.avg_step_freq));
                        });
                        row.col(|ui| {
                            if r.complete {
                                ui.colored_label(theme::ok(), "达标");
                            } else {
                                ui.colored_label(theme::err(), "未达标");
                            }
                        });
                        row.col(|ui| {
                            ui.monospace(rrid.to_string());
                        });
                        row.col(|ui| {
                            let txt = if is_detail {
                                egui::RichText::new("收起").color(theme::accent())
                            } else {
                                egui::RichText::new("详情").color(theme::accent())
                            };
                            if ui.small_button(txt).clicked() {
                                toggle_target.set(if is_detail { None } else { Some(rrid) });
                            }
                        });
                    });
                }
            });

        if let Some(rrid) = toggle_target.get() {
            self.records_page.detail_rrid = Some(rrid);
            self.records_page.detail_raw = None;
            self.records_page.detail_loading = true;
            self.fetch_run_detail(rrid);
        }
    }

    fn draw_ai_table(&mut self, ui: &mut egui::Ui) {
        if self.records_page.ai_groups.is_empty() {
            ui.add_space(20.0);
            ui.centered_and_justified(|ui| ui.label("暂无 AI 记录，选择项目后「刷新记录」"));
            return;
        }
        ui.label(format!(
            "共 {} 条（{} 天）",
            self.records_page.ai_total,
            self.records_page.ai_groups.len()
        ));
        let num = |s: &str| s.parse::<f64>().unwrap_or(0.0);
        TableBuilder::new(ui)
            .striped(true)
            .vscroll(true)
            .auto_shrink([false, false])
            .column(Column::auto().at_least(100.0)) // 日期
            .column(Column::auto().at_least(110.0)) // 项目
            .column(Column::auto().at_least(90.0)) // 成绩
            .column(Column::auto().at_least(100.0)) // 完成时间
            .column(Column::auto().at_least(90.0)) // 提交时间
            .column(Column::remainder()) // 视频
            .header(22.0, |mut h| {
                for t in ["日期", "项目", "成绩", "完成时间", "提交时间", "视频"] {
                    h.col(|ui| {
                        ui.label(egui::RichText::new(t).strong().color(theme::text()));
                    });
                }
            })
            .body(|mut body| {
                for g in &self.records_page.ai_groups {
                    let date = chrono::Local
                        .timestamp_millis_opt(g.score_date)
                        .single()
                        .map(|t| t.format("%Y-%m-%d").to_string())
                        .unwrap_or_default();
                    for r in &g.records {
                        body.row(20.0, |mut row| {
                            row.col(|ui| {
                                ui.monospace(&date);
                            });
                            row.col(|ui| {
                                ui.label(&r.name);
                            });
                            row.col(|ui| {
                                if r.rtype == 2 {
                                    // 计时类：成绩即用时（毫秒）
                                    ui.monospace(format!("{:.1} 秒", num(&r.score) / 1000.0));
                                } else {
                                    ui.monospace(format!("{} 个", r.score));
                                }
                            });
                            row.col(|ui| {
                                let ms = chrono::Local
                                    .timestamp_millis_opt(r.score_date)
                                    .single()
                                    .map(|t| t.format("%H:%M:%S").to_string())
                                    .unwrap_or_default();
                                ui.monospace(ms);
                            });
                            row.col(|ui| {
                                ui.monospace(
                                    chrono::Local
                                        .timestamp_millis_opt(r.upload_time)
                                        .single()
                                        .map(|t| t.format("%m-%d %H:%M").to_string())
                                        .unwrap_or_default(),
                                );
                            });
                            row.col(|ui| {
                                if r.has_video {
                                    ui.colored_label(theme::accent(), "有");
                                } else {
                                    ui.label("-");
                                }
                            });
                        });
                    }
                }
            });
    }
}

fn draw_detail_panel(ui: &mut egui::Ui, raw: &serde_json::Value) {
    let g = |k: &str| -> String {
        match raw.get(k) {
            Some(serde_json::Value::String(s)) if !s.is_empty() => s.clone(),
            Some(serde_json::Value::Number(n)) => n.to_string(),
            _ => String::new(),
        }
    };
    let f = |k: &str| -> f64 { raw.get(k).and_then(|v| v.as_f64()).unwrap_or(0.0) };
    let i = |k: &str| -> i64 { raw.get(k).and_then(|v| v.as_i64()).unwrap_or(0) };

    egui::Grid::new("detail_grid")
        .num_columns(2)
        .spacing([18.0, 4.0])
        .striped(true)
        .show(ui, |ui| {
            let mut items: Vec<(&str, String)> = vec![
                ("距离", format!("{:.2} km", f("totalDis") / 1000.0)),
                ("有效里程", format!("{:.2} km", f("validDis") / 1000.0)),
                ("时长", {
                    let t = i("totalTime");
                    if t >= 3600 {
                        format!("{}:{:02}:{:02}", t / 3600, t % 3600 / 60, t % 60)
                    } else {
                        format!("{}:{:02}", t / 60, t % 60)
                    }
                }),
                ("卡路里", format!("{} kcal", i("calorie"))),
                ("功率", format!("{} W", i("avgPower"))),
                ("步数", i("totalSteps").to_string()),
                ("步频", format!("{} spm", i("avgStepFreq"))),
                ("爬升", format!("{} m", i("totalAscent"))),
                ("配速区间", {
                    let lo = f("speedBottom");
                    let hi = f("speedTop");
                    if hi > 0.0 { format!("{:.2} ~ {:.2} m/s", lo, hi) } else { "-".into() }
                }),
                ("地址", g("address")),
                ("状态", g("statusInfo")),
            ];
            let st_ms = i("startTime");
            if st_ms > 0 {
                let st = chrono::Local
                    .timestamp_millis_opt(st_ms)
                    .single()
                    .map(|t| t.format("%Y-%m-%d %H:%M:%S").to_string())
                    .unwrap_or_default();
                if !st.is_empty() {
                    items.push(("开始时间", st));
                }
            }
            let _ = &mut items;
            for (k, v) in &items {
                if v.is_empty() || v == "0" || v == "0.0" { continue; }
                ui.label(egui::RichText::new(*k).color(theme::text_dim()));
                ui.label(egui::RichText::new(v).color(theme::text()));
                ui.end_row();
            }
        });

    // 达标判定
    if let Some(list) = raw.get("reasonList").and_then(|x| x.as_array()) {
        ui.add_space(6.0);
        ui.label(egui::RichText::new("达标判定").strong().color(theme::text()));
        for r in list {
            let ok = r.get("complete").and_then(|x| x.as_bool()).unwrap_or(false);
            let reason = r.get("reason").and_then(|x| x.as_str()).unwrap_or("");
            ui.horizontal(|ui| {
                if ok {
                    ui.colored_label(theme::ok(), "√");
                } else {
                    ui.colored_label(theme::err(), "×");
                }
                ui.label(reason);
            });
        }
    }
}
