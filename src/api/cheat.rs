//! 违规通报名单：POST {DISCOVERY}/api/v78/cheat/cheatlist。
//! self=null 自己干净；非 null 已被标记（含原因字段）。

use super::client::ApiClient;
use crate::api::model::DISCOVERY;
use serde_json::{json, Value};

pub const CHEATLIST_PATH: &str = "/api/v78/cheat/cheatlist";

pub struct CheatReport {
    /// null=干净。
    pub self_info: Value,
    pub list: Vec<Value>,
}

impl CheatReport {
    pub fn is_clean(&self) -> bool {
        self.self_info.is_null()
    }

    pub fn self_brief(&self) -> String {
        if self.self_info.is_null() {
            return "干净".into();
        }
        let v = &self.self_info;
        let reason = v
            .get("reason")
            .or_else(|| v.get("punishReason"))
            .or_else(|| v.get("cause"))
            .and_then(|r| r.as_str())
            .unwrap_or("");
        let name = v.get("name").and_then(|n| n.as_str()).unwrap_or("");
        format!("{} {}", name, reason).trim().to_string()
    }
}

pub fn query(client: &mut ApiClient, page: i64, log: &mut dyn FnMut(&str)) -> Result<CheatReport, String> {
    let unid: i64 = client
        .login
        .as_ref()
        .map(|s| s.unid.parse().unwrap_or(0))
        .unwrap_or(0);
    let body = json!({ "pageNum": page, "pageSize": 20, "unid": unid }).to_string();
    let biz = client.call_host(DISCOVERY, "POST", CHEATLIST_PATH, &body, &[])?;
    let data = super::client::parse_data_field(&biz);
    let list = data
        .get("list")
        .and_then(|l| l.as_array())
        .cloned()
        .unwrap_or_default();
    let self_info = data.get("self").cloned().unwrap_or(Value::Null);
    log(&format!(
        "[cheat] self={} 全校违规 {} 条",
        if self_info.is_null() { "null" } else { "非空" },
        list.len()
    ));
    Ok(CheatReport { self_info, list })
}
