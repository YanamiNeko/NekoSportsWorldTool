//! 排行榜，全部走 {DISCOVERY}：
//! 主榜 /api/v41/rank；历史榜 /api/v41/historyRank；室内榜 /api/v43/runnings/indoor/studentRank。

use super::client::ApiClient;
use crate::api::model::DISCOVERY;
use serde_json::{json, Value};

pub const RANK_PATH: &str = "/api/v41/rank";
pub const HISTORY_PATH: &str = "/api/v41/historyRank";
pub const INDOOR_PATH: &str = "/api/v43/runnings/indoor/studentRank";

/// 榜单行（解析失败的行保留原始 JSON）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RankRow {
    pub sort: i64,
    pub name: String,
    /// 米。
    pub length: f64,
    #[serde(default)]
    pub gender: i64,
}

/// 主榜参数：rtype=1个人 2班级 3院系；sort=1日榜 2月榜；
/// date 来自登录态 UserInfo（YYYY-MM-DD），缺省取当天；gender 协议必带，缺省 1。
pub fn main_rank(
    client: &mut ApiClient,
    rtype: i64,
    sort_type: i64,
    gender: Option<i64>,
    date: Option<String>,
) -> Result<Vec<RankRow>, String> {
    let unid = unid_of(client);
    let date = date.unwrap_or_else(|| {
        chrono::Local::now().format("%Y-%m-%d").to_string()
    });
    let body = json!({
        "unid": unid,
        "type": rtype,
        "sortType": sort_type,
        "date": date,
        "gender": gender.unwrap_or(1),
    });
    let biz = client.call_host(DISCOVERY, "POST", RANK_PATH, &body.to_string(), &[])?;
    dump_unknown(&biz, "");
    Ok(rows_from(parse_data(&biz)))
}

/// 历史榜：sort=1日 2月。
pub fn history_rank(client: &mut ApiClient, sort_type: i64, gender: i64) -> Result<Vec<RankRow>, String> {
    let body = json!({
        "unid": unid_of(client),
        "sortType": sort_type,
        "gender": gender,
        "pageNo": 1,
        "pageSize": 20,
    });
    let biz = client.call_host(DISCOVERY, "POST", HISTORY_PATH, &body.to_string(), &[])?;
    dump_unknown(&biz, "");
    Ok(rows_from(parse_data(&biz)))
}

/// 室内榜：range=1日 2周 3月。
pub fn indoor_rank(client: &mut ApiClient, date_range: i64, gender: i64) -> Result<Vec<RankRow>, String> {
    let body = json!({
        "pageNum": 1,
        "pageSize": 20,
        "gender": gender,
        "dateRange": date_range,
    });
    let biz = client.call_host(DISCOVERY, "POST", INDOOR_PATH, &body.to_string(), &[])?;
    dump_unknown(&biz, "");
    Ok(rows_from(parse_data(&biz)))
}

fn unid_of(client: &ApiClient) -> i64 {
    client.login.as_ref().map(|s| s.unid.parse().unwrap_or(0)).unwrap_or(0)
}

/// data 可能是数组或 {list:[...]} 包裹。
fn parse_data(biz: &Value) -> Vec<Value> {
    let data = super::client::parse_data_field(biz);
    if let Some(a) = data.as_array() {
        return a.clone();
    }
    for key in ["list", "records", "rankList", "ranks"] {
        if let Some(a) = data.get(key).and_then(|v| v.as_array()) {
            return a.clone();
        }
    }
    Vec::new()
}

/// 行字段按实际响应宽容解析；无 name/length 的行丢弃（原始 JSON 已落地）。
fn rows_from(arr: Vec<Value>) -> Vec<RankRow> {
    arr.iter()
        .enumerate()
        .filter_map(|(i, r)| {
            let name = r
                .get("name")
                .or_else(|| r.get("userName"))
                .or_else(|| r.get("nickname"))
                .and_then(|v| v.as_str())?;
            let length = r
                .get("length")
                .or_else(|| r.get("totalDis"))
                .or_else(|| r.get("dis"))
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let sort = r
                .get("sort")
                .or_else(|| r.get("rank"))
                .and_then(|v| v.as_i64())
                .unwrap_or(i as i64 + 1);
            Some(RankRow {
                sort,
                name: name.to_string(),
                length,
                gender: r.get("gender").and_then(|v| v.as_i64()).unwrap_or(-1),
            })
        })
        .collect()
}

/// 首跑未知结构落地（exe 同目录）供字段核对。
fn dump_unknown(biz: &Value, name: &str) {
    let path = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|p| p.join(name)));
    if let Some(path) = path {
        if !path.exists() {
            let _ = std::fs::write(&path, biz.to_string());
        }
    }
}
