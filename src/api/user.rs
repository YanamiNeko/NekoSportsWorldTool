//! 个人信息。
//! 聚合：登录 profile / v41/user/HomePageInfo / v41/running/getPersonalSemesterInfo(runMode)
//! / v55 recordssummary / v44 personalSemesterCompleted(需 rrid)。

use super::client::{parse_data_field, ApiClient};
use serde_json::{json, Value};

pub const HOME_PAGE_INFO_PATH: &str = "/api/v41/user/HomePageInfo";
pub const PERSONAL_SEMESTER_PATH: &str = "/api/v41/running/getPersonalSemesterInfo";
pub const SEMESTER_COMPLETED_PATH: &str = "/api/v44/runnings/personalSemesterCompleted";

pub struct MyInfo {
    pub errors: Vec<String>,
    pub profile: Value,
    pub home_page: Value,
    pub personal_semester: Value,
    pub summary: Value,
    pub completed: Value,
}

/// 拉取我的页全部数据；单项失败不阻塞其他项。
pub fn fetch_my_info(client: &mut ApiClient, log: &mut dyn FnMut(&str)) -> MyInfo {
    let mut errors = Vec::new();
    let profile = client
        .login
        .as_ref()
        .map(|s| s.profile.clone())
        .unwrap_or(Value::Null);

    let home_page = client
        .call("GET", HOME_PAGE_INFO_PATH, "{}", &[])
        .map(|biz| parse_data_field(&biz))
        .unwrap_or_else(|e| { errors.push(format!("主页：{e}")); Value::Null });
    if home_page.is_null() {
        log("⚠ [user] HomePageInfo 拉取失败");
    }

    // 文档确认：getPersonalSemesterInfo 需要传 runMode
    let personal_semester = client
        .call(
            "POST",
            PERSONAL_SEMESTER_PATH,
            &json!({ "runMode": 1 }).to_string(),
            &[],
        )
        .map(|biz| parse_data_field(&biz))
        .unwrap_or_else(|e| { errors.push(e); Value::Null });

    let summary = client
        .call(
            "POST",
            super::semester::SUMMARY_PATH,
            &json!({}).to_string(),
            &[],
        )
        .map(|biz| parse_data_field(&biz))
        .unwrap_or_else(|e| { errors.push(e); Value::Null });

    // 学期完成状态需要最近一条记录的 rrid
    let completed = match super::records::fetch_records(client) {
        Ok(rows) if !rows.is_empty() => client
            .call(
                "POST",
                SEMESTER_COMPLETED_PATH,
                &json!({ "rrid": rows[0].rrid }).to_string(),
                &[],
            )
            .map(|biz| parse_data_field(&biz))
            .unwrap_or_else(|e| { errors.push(format!("完成状态：{e}")); Value::Null }),
        Err(e) => { errors.push(format!("最近记录：{e}")); Value::Null },
        _ => Value::Null,
    };

    MyInfo { errors, profile, home_page, personal_semester, summary, completed }
}


impl MyInfo {
    pub fn refresh_message(&self) -> String {
        if !self.errors.is_empty() {
            format!("⚠ [user] 个人数据刷新未全部成功：{}", self.errors.join("；"))
        } else if [&self.home_page, &self.personal_semester, &self.summary, &self.completed].iter().all(|v| v.is_null()) {
            "[user] 未获取到新的个人数据；登录资料仅为缓存".into()
        } else {
            "√ [user] 我的页数据已更新".into()
        }
    }
}

#[cfg(test)]
mod refresh_tests {
    use super::*;
    #[test]
    fn cached_profile_and_failed_reads_cannot_claim_a_successful_refresh() {
        let mut info = MyInfo { errors: vec![], profile: json!({"name":"cached"}), home_page: Value::Null, personal_semester: Value::Null, summary: Value::Null, completed: Value::Null };
        assert!(!info.refresh_message().contains("已更新"));
        info.home_page = json!({"count":1});
        info.errors.push("请求失败".into());
        assert!(!info.refresh_message().contains("已更新"));
        info.errors.clear();
        assert!(info.refresh_message().contains("已更新"));
    }
}
