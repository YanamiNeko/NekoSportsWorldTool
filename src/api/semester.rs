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
    #[serde(default)]
    pub completed_proportion: Option<Value>,
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
        completed_proportion: data.get("completedProportion").filter(|v| v.is_string() || v.is_number()).cloned(),
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

    merge_explicit(&mut summary, &personal_raw);

    // 仅合并名称明确的字段，不把目标次数作为历史总次数。
    if summary.completed_proportion.is_none() || summary.semester_valid_dis == 0.0 {
        let rrid = super::records::fetch_records(client)
            .ok()
            .and_then(|rows| rows.first().map(|r| r.rrid));
        if let Some(rrid) = rrid {
            let body = json!({ "rrid": rrid }).to_string();
            if let Ok(biz) = client.call("POST", super::user::SEMESTER_COMPLETED_PATH, &body, &[]) {
                let done = super::client::parse_data_field(&biz);
                merge_explicit(&mut summary, &done);
                if let Some(n) = done.get("semesterCount").and_then(|v| v.as_i64()).filter(|n| *n >= 0) {
                    summary.semester_count = n;
                }
            }
        }
    }

    Ok(SemesterResult { summary: Some(summary), personal_raw })
}


fn merge_explicit(summary: &mut SemesterSummary, data: &Value) {
    if summary.semester_valid_dis == 0.0 {
        if let Some(n) = data.get("semesterValidDis").and_then(Value::as_f64).filter(|n| *n >= 0.0) {
            summary.semester_valid_dis = n;
        }
    }
    if summary.semester_valid_count == 0 {
        if let Some(n) = data.get("semesterValidCount").and_then(Value::as_i64).filter(|n| *n >= 0) {
            summary.semester_valid_count = n;
        }
    }
    if summary.completed_proportion.is_none() {
        summary.completed_proportion = data.get("completedProportion").filter(|v| v.is_string() || v.is_number()).cloned();
    }
}
impl SemesterSummary {
    pub fn display(&self) -> String {
        let completion = self.completed_proportion.as_ref().map(|v| v.as_str().map(str::to_owned).unwrap_or_else(|| v.to_string())).unwrap_or_else(|| "未知".into());
        format!("学期 {}：有效 {} 次（总 {} 次），有效里程 {:.2} km（总 {:.2} km）；完成度（服务端原值）：{}",
            self.sname, self.semester_valid_count, self.semester_count, self.semester_valid_dis / 1000.0, self.semester_dis / 1000.0, completion)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn distance_never_becomes_count_or_target() {
        let mut s = SemesterSummary { sname: "测试".into(), semester_dis: 42270.0, semester_valid_dis: 0.0, semester_count: 12, semester_valid_count: 0, completed_proportion: None };
        merge_explicit(&mut s, &json!({"semesterValid":32000,"semesterGoalCount":60}));
        assert_eq!(s.semester_valid_count,0);
        assert_eq!(s.semester_count,12);
        merge_explicit(&mut s, &json!({"semesterValidDis":32000,"semesterValidCount":10,"completedProportion":"40%"}));
        assert_eq!(s.semester_valid_count,10);
        assert!(s.display().contains("32.00 km"));
        assert!(s.display().contains("40%"));
    }
}
