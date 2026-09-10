//! 个人信息。
//! 聚合：登录 profile / v41/user/HomePageInfo / v41/running/getPersonalSemesterInfo(runMode)
//! / v55 recordssummary / v44 personalSemesterCompleted(需 rrid)。

use super::client::{parse_data_field, ApiClient};
use serde_json::{json, Value};

pub const HOME_PAGE_INFO_PATH: &str = "/api/v41/user/HomePageInfo";
pub const PERSONAL_SEMESTER_PATH: &str = "/api/v41/running/getPersonalSemesterInfo";
pub const SEMESTER_COMPLETED_PATH: &str = "/api/v44/runnings/personalSemesterCompleted";

pub struct MyInfo {
    pub profile: Value,
    pub home_page: Value,
    pub personal_semester: Value,
    pub summary: Value,
    pub completed: Value,
}

/// 拉取我的页全部数据；单项失败不阻塞其他项。
pub fn fetch_my_info(client: &mut ApiClient, log: &mut dyn FnMut(&str)) -> MyInfo {
    let profile = client
        .login
        .as_ref()
        .map(|s| s.profile.clone())
        .unwrap_or(Value::Null);

    let home_page = client
        .call("GET", HOME_PAGE_INFO_PATH, "{}", &[])
        .map(|biz| parse_data_field(&biz))
        .unwrap_or(Value::Null);
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
        .unwrap_or(Value::Null);

    let summary = client
        .call(
            "POST",
            super::semester::SUMMARY_PATH,
            &json!({}).to_string(),
            &[],
        )
        .map(|biz| parse_data_field(&biz))
        .unwrap_or(Value::Null);

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
            .unwrap_or(Value::Null),
        _ => Value::Null,
    };

    MyInfo { profile, home_page, personal_semester, summary, completed }
}
