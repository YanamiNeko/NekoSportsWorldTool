//! 数据页：学期完成度 / 违规自查 / 排行榜。

use super::{theme, App};
use chrono::TimeZone;
use crate::api::cheat::CheatReport;
use crate::api::rank::RankRow;
use crate::api::semester::SemesterSummary;
use eframe::egui;

#[derive(Default)]
pub struct DataPage {
    pub semester: Option<SemesterSummary>,
    pub cheat: Option<CheatReport>,
    pub rank_rows: Vec<RankRow>,
    /// 0=个人日 1=个人月 2=班级日 3=班级月 4=院系日 5=院系月 6=室内日 7=室内周 8=室内月 9=历史日 10=历史月
    pub rank_sel: usize,
    pub show_cheat_list: bool,
}

pub const RANK_OPTIONS: [&str; 11] = [
    "个人日榜", "个人月榜", "班级日榜", "班级月榜", "院系日榜", "院系月榜",
    "室内日榜", "室内周榜", "室内月榜", "历史日榜", "历史月榜",
];

impl DataPage {
    /// 榜单选择 → (kind, type参数)。
    pub fn selection(&self) -> (&'static str, i64) {
        match self.rank_sel {
            0 => ("main", 11),  // type=1 sort=1
            1 => ("main", 12),
            2 => ("main", 21),
            3 => ("main", 22),
            4 => ("main", 31),
            5 => ("main", 32),
            6 => ("indoor", 1),
            7 => ("indoor", 2),
            8 => ("indoor", 3),
            9 => ("history", 1),
            _ => ("history", 2),
        }
    }
}

impl App {
    pub fn draw_data(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);

        // ── 学期完成度 ──────────────────────────────────────────
        egui::CollapsingHeader::new("学期完成度")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    match &self.data_page.semester {
                        Some(s) => {
                            let km = |m: f64| format!("{:.2}", m / 1000.0);
                            ui.label(format!("学期：{}", s.sname));
                            ui.separator();
                            ui.label(format!(
                                "有效次数：{}/{}",
                                s.semester_valid_count, s.semester_count
                            ));
                            ui.separator();
                            ui.label(format!(
                                "有效里程：{}/{} km（总 {} km）",
                                km(s.semester_valid_dis),
                                km(s.semester_dis),
                                km(s.semester_dis)
                            ));
                        }
                        None => {
                            ui.label("未拉取");
                        }
                    }
                    if ui.add_enabled(!self.data_busy, theme::primary_btn("刷新")).clicked() {
                        self.refresh_data_page();
                    }
                });
            });

        // ── 违规自查 ────────────────────────────────────────────
        egui::CollapsingHeader::new("违规自查")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    match &self.data_page.cheat {
                        Some(c) => {
                            if c.is_clean() {
                                ui.colored_label(
                                    theme::ok(),
                                    "自查：正常（self=null）",
                                );
                            } else {
                                ui.colored_label(
                                    theme::err(),
                                    format!("已被标记：{}", c.self_brief()),
                                );
                            }
                            ui.separator();
                            ui.label(format!("全校违规：{} 条", c.list.len()));
                            ui.toggle_value(&mut self.data_page.show_cheat_list, "查看列表");
                        }
                        None => {
                            ui.label("未检查");
                        }
                    }
                    if ui.add_enabled(!self.data_busy, theme::primary_btn("立即检查")).clicked() {
                        self.refresh_cheat_only();
                    }
                });
                if self.data_page.show_cheat_list {
                    if let Some(c) = &self.data_page.cheat {
                        egui::ScrollArea::vertical()
                            .max_height(160.0)
                            .show(ui, |ui| {
                                egui::Grid::new("cheat_grid")
                                    .num_columns(4)
                                    .striped(true)
                                    .show(ui, |ui| {
                                        ui.strong("姓名");
                                        ui.strong("原因");
                                        ui.strong("时间");
                                        ui.strong("unid");
                                        ui.end_row();
                                        for r in &c.list {
                                            ui.label(str_of(r, &["name", "userName", "studentName"]));
                                            ui.label(str_of(
                                                r,
                                                &["reason", "punishReason", "cause", "type"],
                                            ));
                                            ui.label(time_of(r, &["createTime", "time", "date"]));
                                            ui.label(str_of(r, &["unid", "sid"]));
                                            ui.end_row();
                                        }
                                    });
                            });
                    }
                }
            });

        // ── 排行榜 ──────────────────────────────────────────────
        egui::CollapsingHeader::new("排行榜")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    egui::ComboBox::from_id_salt("rank_combo")
                        .selected_text(RANK_OPTIONS[self.data_page.rank_sel])
                        .show_ui(ui, |ui| {
                            for (i, opt) in RANK_OPTIONS.iter().enumerate() {
                                ui.selectable_value(&mut self.data_page.rank_sel, i, *opt);
                            }
                        });
                    if ui.add_enabled(!self.data_busy, theme::primary_btn("查询")).clicked() {
                        let (kind, subtype) = self.data_page.selection();
                        self.refresh_rank(kind, subtype);
                    }
                    if self.data_busy {
                        ui.label("拉取中…");
                    }
                });
                ui.add_space(4.0);
                if self.data_page.rank_rows.is_empty() {
                    ui.label("（无数据）");
                } else {
                    egui::ScrollArea::vertical()
                        .max_height(240.0)
                        .show(ui, |ui| {
                            egui::Grid::new("rank_grid")
                                .num_columns(4)
                                .striped(true)
                                .show(ui, |ui| {
                                    ui.strong("名次");
                                    ui.strong("姓名");
                                    ui.strong("里程");
                                    ui.strong("性别");
                                    ui.end_row();
                                    for r in &self.data_page.rank_rows {
                                        ui.monospace(r.sort.to_string());
                                        ui.label(&r.name);
                                        ui.monospace(format!("{:.2} km", r.length / 1000.0));
                                        ui.label(match r.gender {
                                            1 => "男",
                                            0 => "女",
                                            _ => "-",
                                        });
                                        ui.end_row();
                                    }
                                });
                        });
                }
            });
    }
}

fn str_of(v: &serde_json::Value, keys: &[&str]) -> String {
    for k in keys {
        if let Some(hit) = v.get(k) {
            match hit {
                serde_json::Value::String(s) => return s.clone(),
                serde_json::Value::Number(n) => return n.to_string(),
                _ => {}
            }
        }
    }
    "-".into()
}

/// 时间列：毫秒时间戳 → 可读时间，字符串原样返回。
fn time_of(v: &serde_json::Value, keys: &[&str]) -> String {
    for k in keys {
        if let Some(hit) = v.get(k) {
            match hit {
                serde_json::Value::Number(n) => {
                    if let Some(ms) = n.as_i64() {
                        if ms > 100_000_000_000 {
                            return chrono::Local
                                .timestamp_millis_opt(ms)
                                .single()
                                .map(|t| t.format("%Y-%m-%d %H:%M").to_string())
                                .unwrap_or_else(|| n.to_string());
                        }
                    }
                    return n.to_string();
                }
                serde_json::Value::String(s) if !s.is_empty() => return s.clone(),
                _ => {}
            }
        }
    }
    "-".into()
}

