//! 我的页：个人信息 + 学期 + 完成状态 + 主页统计。
//! 数据来源：登录 profile / HomePageInfo / getPersonalSemesterInfo(runMode) /
//! recordssummary / personalSemesterCompleted(最近 rrid)。

use super::{theme, App};
use eframe::egui;
use egui::RichText;
use chrono::TimeZone;
use serde_json::Value;

#[derive(Default)]
pub struct UserPage {
    pub info: Option<crate::api::user::MyInfo>,
}

impl App {
    pub fn draw_user(&mut self, ui: &mut egui::Ui) {
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(!self.user_busy, theme::primary_btn("刷新"))
                .clicked()
            {
                self.refresh_user_page();
            }
            if self.user_busy {
                ui.label("拉取中…");
            }
        });
        ui.separator();

        let Some(info) = &self.user_page.info else {
            ui.add_space(12.0);
            ui.label("未拉取，点击「刷新」");
            return;
        };

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                section(ui, "个人信息", |ui| {
                    let rows = profile_rows(&info.profile);
                    if rows.is_empty() {
                        ui.label("本次登录未返回 profile，重新登录后可见");
                    } else {
                        grid(ui, "profile_grid", &rows);
                    }
                });

                section(ui, "学期", |ui| {
                    grid(ui, "sem_grid", &localized(&info.personal_semester));
                });

                section(ui, "完成状态", |ui| {
                    if info.completed.is_null() {
                        ui.label("（暂无有效记录）");
                    } else {
                        grid(ui, "done_grid", &localized(&info.completed));
                    }
                });

                section(ui, "主页统计", |ui| {
                    if info.home_page.is_null() {
                        ui.label("（HomePageInfo 不可用）");
                    } else {
                        grid(ui, "home_grid", &localized(&info.home_page));
                    }
                });

                section(ui, "学期汇总", |ui| {
                    grid(ui, "sum_grid", &localized(&info.summary));
                });
            });
    }
}

fn section(ui: &mut egui::Ui, title: &str, add: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(RichText::new(title).strong().color(theme::text()))
        .default_open(true)
        .show(ui, add);
}

fn grid(ui: &mut egui::Ui, id: &str, rows: &[(String, String)]) {
    egui::Grid::new(id)
        .num_columns(2)
        .spacing([18.0, 5.0])
        .striped(true)
        .show(ui, |ui| {
            for (k, v) in rows {
                ui.label(RichText::new(k).color(theme::text_dim()));
                ui.label(RichText::new(v).color(theme::plain()));
                ui.end_row();
            }
        });
}

/// 字段名中文化（顺带单位换算）；未知字段与无意义的 0 不展示。
const FIELD_LABELS: &[(&str, &str)] = &[
    ("sid", "学生 ID"),
    ("sname", "学期"),
    ("startDate", "学期开始"),
    ("endDate", "学期结束"),
    ("semesterGoal", "学期目标"),
    ("semesterGoalCount", "目标次数"),
    ("semesterValid", "已有效次数"),
    ("semesterValidCount", "有效次数"),
    ("semesterValidDis", "有效里程"),
    ("curValidDis", "当前有效里程"),
    ("requiredDays", "要求天数"),
    ("goalMethod", "考核方式"),
    ("completedProportion", "完成度 %"),
    ("defeatProportion", "击败同学 %"),
    ("collectionNum", "收藏"),
    ("fansNum", "粉丝"),
    ("interestNum", "关注"),
    ("point", "积分"),
    ("praiseNum", "获赞"),
    ("rank", "排名"),
    ("rankName", "称号"),
    ("runCount", "次数"),
    ("semesterRunLength", "里程"),
    ("morningValidTotalCount", "晨跑有效次数"),
    ("morningValidTotalDis", "晨跑有效里程"),
    ("validTotalCount", "有效总次数"),
    ("validTotalDis", "有效总里程"),
];

/// 里程类字段（米 → 千米展示）。
const METRE_FIELDS: &[&str] = &[
    "semesterValidDis",
    "curValidDis",
    "semesterRunLength",
    "validTotalDis",
];

/// 毫秒时间戳字段（→ 日期展示）。
const TIME_FIELDS: &[&str] = &["startDate", "endDate"];

fn fmt_day(ms: i64) -> Option<String> {
    chrono::Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|t| t.format("%Y-%m-%d").to_string())
}

fn localized(v: &Value) -> Vec<(String, String)> {
    let Some(obj) = v.as_object() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for (key, label) in FIELD_LABELS {
        let Some(val) = obj.get(*key) else { continue };
        if val.is_null() {
            continue;
        }
        if val.as_f64().map(|f| f == 0.0).unwrap_or(false) {
            continue;
        }
        if let Some(s) = val.as_str() {
            if s.is_empty() {
                continue;
            }
        }
        let text = if METRE_FIELDS.contains(key) {
            match val.as_f64() {
                Some(m) => format!("{:.2} km", m / 1000.0),
                None => val.to_string(),
            }
        } else if TIME_FIELDS.contains(key) {
            match val.as_i64().and_then(fmt_day) {
                Some(d) => d,
                None => val.to_string(),
            }
        } else if *key == "goalMethod" {
            match val.as_i64() {
                Some(1) => "里程".into(),
                Some(2) => "次数".into(),
                _ => val.to_string(),
            }
        } else {
            match val {
                Value::String(s) => s.clone(),
                other => other.to_string(),
            }
        };
        out.push((label.to_string(), text));
    }
    out
}

const PROFILE_KEYS: &[(&str, &str)] = &[
    ("uid", "uid"),
    ("name", "姓名"),
    ("username", "账号"),
    ("telephone", "手机号"),
    ("sex", "性别"),
    ("weight", "体重 kg"),
    ("height", "身高 cm"),
    ("birthday", "生日"),
    ("unid", "学校 unid"),
    ("campusName", "学校"),
    ("depart", "院系"),
    ("gradeClass", "班级"),
    ("enrollmentYear", "入学年份"),
    ("email", "邮箱"),
    ("nickname", "昵称"),
    ("roleName", "角色"),
    ("point", "积分"),
    ("alias", "别名"),
    ("endTime", "账号有效期"),
    ("campusId", "校区 ID"),
    ("classId", "班级 ID"),
];

fn profile_rows(profile: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (key, label) in PROFILE_KEYS {
        let Some(val) = profile.get(key) else { continue };
        let text = match val {
            Value::String(s) if !s.is_empty() => s.clone(),
            Value::Number(n) if n.as_f64() != Some(0.0) => n.to_string(),
            _ => continue,
        };
        let text = if *key == "endTime" {
            // 账号有效期：毫秒时间戳 → 日期
            val.as_i64()
                .and_then(|ms| {
                    chrono::Local
                        .timestamp_millis_opt(ms)
                        .single()
                        .map(|t| t.format("%Y-%m-%d").to_string())
                })
                .unwrap_or(text)
        } else if *key == "sex" {
            match val.as_i64() {
                Some(1) => "男".into(),
                Some(2) => "女".into(),
                _ => text,
            }
        } else {
            text
        };
        out.push((label.to_string(), text));
    }
    out
}
