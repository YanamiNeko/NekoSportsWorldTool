//! AI 运动。
//! 列表：GET /api/v1/sport/ai/list {}；提交：POST /api/v65/sport/ai/record/upload；
//! 记录：GET /api/v66/sport/ai/record/infos?sportId=&pageSize=&pageNum=。

use super::client::{get_field, parse_data_field, ApiClient};
use serde_json::{json, Value};

pub const AI_LIST_PATH: &str = "/api/v1/sport/ai/list";
pub const AI_UPLOAD_PATH: &str = "/api/v65/sport/ai/record/upload";
#[allow(dead_code)]
pub const AI_RECORDS_PATH: &str = "/api/v66/sport/ai/record/infos";

/// AI 运动项目。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AiSport {
    pub id: i64,
    pub name: String,
}

/// 拉取项目列表；成功即落盘永久缓存，失败时回退缓存。
pub fn fetch_list(client: &mut ApiClient) -> Result<Vec<AiSport>, String> {
    match fetch_list_remote(client) {
        Ok(list) => {
            let _ = crate::api::model::save_ai_sports(&list);
            Ok(list)
        }
        Err(e) => match crate::api::model::load_ai_sports() {
            Some(cached) if !cached.is_empty() => Ok(cached),
            _ => Err(e),
        },
    }
}

fn fetch_list_remote(client: &mut ApiClient) -> Result<Vec<AiSport>, String> {
    let biz = client.call("GET", AI_LIST_PATH, "{}", &[])?;
    let arr = get_field(&biz, "list")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    Ok(arr
        .iter()
        .filter_map(|it| {
            Some(AiSport {
                id: it.get("id")?.as_i64()?,
                name: it.get("name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
            })
        })
        .collect())
}

/// 提交模式。
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum AiMode {
    /// 按分钟：score = 用时（毫秒），1-30 分钟
    Task { score_ms: i64, task_id: i64 },
    /// 按次：score = 个数，5-1000（步长 5）
    Count { reps: i64 },
}

/// 提交 AI 运动记录（task/count 双模式）。
/// at=None 时按自然时序（提交时刻减去用时）；补签传入当天目标时刻。
pub fn upload(client: &mut ApiClient, sport_id: i64, mode: AiMode, at: Option<i64>) -> Result<Value, String> {
    let now = crate::crypto::envelope::now_ms();
    let body = match mode {
        AiMode::Task { score_ms, task_id } => {
            // task：score=用时毫秒；speed=每分钟个数(50 个/时长)；consume≈0.2kcal/秒
            let speed = (50.0 * 60_000.0 / score_ms as f64).round() as i64;
            let consume = (score_ms as f64 / 1000.0 * 0.2 * 10.0).round() / 10.0;
            json!({
                "sportId": sport_id,
                "type": 2,
                "score": score_ms.to_string(),
                "timeConsume": score_ms,
                "speed": speed.to_string(),
                "consume": format!("{consume:.1}"),
                "scoreDate": at.unwrap_or(now - score_ms),
                "uuid": uuid::Uuid::new_v4().to_string(),
                "taskId": task_id,
            })
        }
        AiMode::Count { reps } => {
            // count：score=个数；用时 ~0.7s/个 起步 30s
            let time_consume = (reps * 700).max(30_000);
            let speed = (reps as f64 / (time_consume as f64 / 1000.0) * 60.0).round() as i64;
            let consume = (reps as f64 * 0.2 * 10.0).round() / 10.0;
            json!({
                "sportId": sport_id,
                "type": 1,
                "score": reps.to_string(),
                "timeConsume": time_consume,
                "speed": speed.to_string(),
                "consume": format!("{consume:.1}"),
                "scoreDate": at.unwrap_or(now - time_consume),
                "uuid": uuid::Uuid::new_v4().to_string(),
                "taskId": 0,
            })
        }
    }
    .to_string();
    let biz = client.call("POST", AI_UPLOAD_PATH, &body, &[])?;
    Ok(biz)
}

/// AI 记录（v66 按天分组：list[]{scoreDate, frequency, recordInfos[]}）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AiRecordItem {
    pub id: i64,
    pub name: String,
    pub score: String,
    #[serde(rename = "type")]
    pub rtype: i64,
    pub upload_time: i64,
    pub score_date: i64,
    pub status: i64,
    pub has_video: bool,
    pub time_consume: i64,
    pub speed: String,
    pub consume: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AiRecordGroup {
    pub score_date: i64,
    pub frequency: i64,
    pub records: Vec<AiRecordItem>,
}

/// 项目记录分页总数。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AiRecordPage {
    pub groups: Vec<AiRecordGroup>,
    pub total_count: i64,
}

/// 拉取某项目的 AI 记录（pageSize 条，按天分组）。
pub fn fetch_records(client: &mut ApiClient, sport_id: i64, page_size: i64) -> Result<AiRecordPage, String> {
    let path = format!(
        "{AI_RECORDS_PATH}?sportId={sport_id}&pageSize={page_size}&pageNum=1"
    );
    let biz = client.call("GET", &path, "{}", &[])?;
    let data = parse_data_field(&biz);
    let arr = data
        .get("list")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let groups = arr
        .iter()
        .map(|g| AiRecordGroup {
            score_date: g.get("scoreDate").and_then(|v| v.as_i64()).unwrap_or(0),
            frequency: g.get("frequency").and_then(|v| v.as_i64()).unwrap_or(0),
            records: g
                .get("recordInfos")
                .and_then(|v| v.as_array())
                .map(|items| {
                    items
                        .iter()
                        .map(|r| AiRecordItem {
                            id: r.get("id").and_then(|v| v.as_i64()).unwrap_or(0),
                            name: r.get("name").and_then(|v| v.as_str()).unwrap_or("").into(),
                            score: match r.get("score") {
                                Some(Value::String(s)) => s.clone(),
                                Some(Value::Number(n)) => n.to_string(),
                                _ => String::new(),
                            },
                            rtype: r.get("type").and_then(|v| v.as_i64()).unwrap_or(0),
                            upload_time: r.get("uploadTime").and_then(|v| v.as_i64()).unwrap_or(0),
                            score_date: r.get("scoreDate").and_then(|v| v.as_i64()).unwrap_or(0),
                            status: r.get("status").and_then(|v| v.as_i64()).unwrap_or(0),
                            has_video: r
                                .get("mediaUrl")
                                .or_else(|| r.get("exerciseMediaUrl"))
                                .map(|v| !v.is_null() && v.as_str().map(|s| !s.is_empty()).unwrap_or(false))
                                .unwrap_or(false),
                            time_consume: r.get("timeConsume").and_then(|v| v.as_i64()).unwrap_or(0),
                            speed: match r.get("speed") {
                                Some(Value::String(s)) => s.clone(),
                                Some(Value::Number(n)) => n.to_string(),
                                _ => String::new(),
                            },
                            consume: match r.get("consume") {
                                Some(Value::String(s)) => s.clone(),
                                Some(Value::Number(n)) => n.to_string(),
                                _ => String::new(),
                            },
                        })
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect();
    let total_count = data
        .get("totalCount")
        .and_then(|v| v.as_str().and_then(|s| s.parse().ok()).or_else(|| v.as_i64()))
        .unwrap_or(0);
    Ok(AiRecordPage { groups, total_count })
}
