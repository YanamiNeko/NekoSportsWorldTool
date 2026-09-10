//! 学期完成度。
//! 汇总：POST /api/v55/runnings/recordssummary/semester {}；
//! 个人完成度：POST /api/v41/running/getPersonalSemesterInfo {}（结构按实际响应落地）。

use super::client::ApiClient;
use serde_json::{json, Value};

pub const SUMMARY_PATH: &str = "/api/v55/runnings/recordssummary/semester";
pub const PERSONAL_PATH: &str = "/api/v41/running/getPersonalSemesterInfo";

/// 学期汇总（B1 实测字段）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SemesterSummary {
    pub sname: String,
    pub semester_dis: f64,
    pub semester_valid_dis: f64,
    pub semester_count: i64,
    pub semester_valid_count: i64,
}

#[allow(dead_code)]
pub struct SemesterResult {
    pub summary: Option<SemesterSummary>,
    /// getPersonalSemesterInfo 原始 data（结构未定型，全量落地供核对）。
    pub personal_raw: Value,
}

pub fn query(client: &mut ApiClient, log: &mut dyn FnMut(&str)) -> Result<SemesterResult, String> {
    // 汇总
    let biz = client.call("POST", SUMMARY_PATH, "{}", &[])?;
    let data = super::client::parse_data_field(&biz);
    let mut summary = SemesterSummary {
        sname: data.get("sname").and_then(|v| v.as_str()).unwrap_or("").to_string(),
        semester_dis: data.get("semesterDis").and_then(|v| v.as_f64()).unwrap_or(0.0),
        semester_valid_dis: data.get("semesterValidDis").and_then(|v| v.as_f64()).unwrap_or(0.0),
        semester_count: data.get("semesterCount").and_then(|v| v.as_i64()).unwrap_or(0),
        semester_valid_count: data.get("semesterValidCount").and_then(|v| v.as_i64()).unwrap_or(0),
    };

    // 个人完成度：学期起止 / 考核方式 / 有效次数
    let personal_raw = match client.call("POST", PERSONAL_PATH, &json!({ "runMode": 1 }).to_string(), &[]) {
        Ok(biz) => super::client::parse_data_field(&biz),
        Err(e) => {
            log(&format!("[semester] 个人完成度接口失败（不影响汇总）: {e}"));
            Value::Null
        }
    };
    if summary.sname.is_empty() {
        summary.sname = personal_raw
            .get("sname")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
    }

    // 部分学校汇总接口计数恒为 0，有效次数退回个人完成度接口
    if summary.semester_valid_count == 0 {
        if let Some(n) = personal_raw.get("semesterValid").and_then(|v| v.as_i64()) {
            summary.semester_valid_count = n;
        }
    }

    // 目标次数仅存在于完成状态接口（需最近一条记录的 rrid）
    if summary.semester_count == 0 {
        let rrid = super::records::fetch_records(client)
            .ok()
            .and_then(|rows| rows.first().map(|r| r.rrid));
        if let Some(rrid) = rrid {
            let body = json!({ "rrid": rrid }).to_string();
            if let Ok(biz) = client.call("POST", super::user::SEMESTER_COMPLETED_PATH, &body, &[]) {
                let done = super::client::parse_data_field(&biz);
                if let Some(n) = done.get("semesterGoalCount").and_then(|v| v.as_i64()) {
                    summary.semester_count = n;
                }
                if summary.semester_valid_count == 0 {
                    if let Some(n) = done.get("semesterValidCount").and_then(|v| v.as_i64()) {
                        summary.semester_valid_count = n;
                    }
                }
            }
        }
    }

    Ok(SemesterResult { summary: Some(summary), personal_raw })
}
