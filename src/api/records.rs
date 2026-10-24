//! 跑步记录。
//! 列表：POST /api/v70230/runnings/records {}；单条：POST /api/v70260/runnings/get_one_record。

use super::client::{get_field, parse_data_field, ApiClient};
use serde_json::{json, Value};

pub const RECORDS_PATH: &str = "/api/v70230/runnings/records";
pub const GET_ONE_PATH: &str = "/api/v70260/runnings/get_one_record";

/// 记录列表行（表格展示用）。
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RecordRow {
    pub rrid: i64,
    pub total_dis: f64,
    pub total_time: i64,
    pub start_time: i64,
    pub complete: bool,
    pub avg_step_freq: i64,
    #[serde(default)]
    pub calorie: i64,
    #[serde(default)]
    pub avg_power: i64,
    #[serde(default)]
    pub total_steps: i64,
    #[serde(default)]
    pub uuid: String,
}

/// 拉取记录列表并归一化为行。
pub fn fetch_records(client: &mut ApiClient) -> Result<Vec<RecordRow>, String> {
    let biz = client.call("POST", RECORDS_PATH, "{}", &[])?;
    let arr: Vec<Value> = match parse_data_field(&biz) {
        Value::Array(a) => a,
        Value::Object(ref o) => o
            .get("list")
            .or_else(|| o.get("records"))
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default(),
        _ => get_field(&biz, "list")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default(),
    };
    Ok(arr
        .iter()
        .map(|r| RecordRow {
            rrid: r.get("rrid").and_then(|v| v.as_i64().or_else(|| v.as_str()?.parse().ok())).unwrap_or(0),
            total_dis: r.get("totalDis").and_then(|v| v.as_f64()).unwrap_or(0.0),
            total_time: r.get("totalTime").and_then(|v| v.as_i64()).unwrap_or(0),
            start_time: r.get("startTime").and_then(|v| v.as_i64()).unwrap_or(0),
            complete: r.get("complete").and_then(|v| v.as_bool()).unwrap_or(false),
            avg_step_freq: r.get("avgStepFreq").and_then(|v| v.as_i64()).unwrap_or(0),
            calorie: r.get("calorie").and_then(|v| v.as_i64()).unwrap_or(0),
            avg_power: r.get("avgPower").and_then(|v| v.as_i64()).unwrap_or(0),
            total_steps: r.get("totalSteps").and_then(|v| v.as_i64()).unwrap_or(0),
            uuid: r.get("uuid").and_then(|v| v.as_str()).unwrap_or("").into(),
        })
        .collect())
}

/// 单条详情（验证提交结果用）：返回内层 data 对象。
pub fn fetch_one_record(client: &mut ApiClient, rrid: i64) -> Result<Value, String> {
    let body = json!({ "rrid": rrid, "uuid": Value::Null, "calculateBadge": false }).to_string();
    let biz = client.call("POST", GET_ONE_PATH, &body, &[])?;
    // business.data 可能是 JSON 字符串或对象，其内层再取 data
    let inner = parse_data_field(&biz);
    let record = if inner.get("data").is_some() {
        parse_data_field(&inner)
    } else {
        inner
    };
    if !record.is_object() {
        return Err("记录详情为空或格式异常".into());
    }
    if let Some(id) = record.get("rrid") {
        if id.as_i64().or_else(|| id.as_str()?.parse().ok()) != Some(rrid) {
            return Err("详情返回的记录编号与请求不一致，请刷新重试".into());
        }
    }
    Ok(record)
}

/// 单条跑步详情（get_one_record 返回的完整 data）。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunDetail {
    pub rrid: i64,
    pub uuid: String,
    pub sport_type: i64,
    pub total_time: i64,
    pub total_dis: f64,
    pub speed: f64,
    pub calorie: i64,
    pub avg_power: i64,
    pub total_steps: i64,
    pub total_ascent: i64,
    pub total_descent: i64,
    pub avg_step_freq: i64,
    pub valid_dis: f64,
    pub valid_time: i64,
    pub address: String,
    pub status_info: String,
    pub complete: bool,
    pub reason_list: Vec<ReasonItem>,
    pub start_time: i64,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReasonItem {
    pub reason: String,
    pub complete: bool,
}

/// 拉取单条详情并解析为结构体。
pub fn fetch_detail(client: &mut ApiClient, rrid: i64) -> Result<RunDetail, String> {
    let raw = fetch_one_record(client, rrid)?;
    if let Some(id) = raw.get("rrid") {
        if id.as_i64().or_else(|| id.as_str()?.parse().ok()) != Some(rrid) {
            return Err("详情返回的记录编号与请求不一致，请刷新重试".into());
        }
    }
    let reasons: Vec<ReasonItem> = raw
        .get("reasonList")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter()
                .map(|r| ReasonItem {
                    reason: r.get("reason").and_then(|v| v.as_str()).unwrap_or("").into(),
                    complete: r.get("complete").and_then(|v| v.as_bool()).unwrap_or(false),
                })
                .collect()
        })
        .unwrap_or_default();
    Ok(RunDetail {
        rrid,
        uuid: raw.get("uuid").and_then(|v| v.as_str()).unwrap_or("").into(),
        sport_type: raw.get("sportType").and_then(|v| v.as_i64()).unwrap_or(0),
        total_time: raw.get("totalTime").and_then(|v| v.as_f64()).unwrap_or(0.0) as i64,
        total_dis: raw.get("totalDis").and_then(|v| v.as_f64()).unwrap_or(0.0),
        speed: raw.get("speed").and_then(|v| v.as_f64()).unwrap_or(0.0),
        calorie: raw.get("calorie").and_then(|v| v.as_i64()).unwrap_or(0),
        avg_power: raw.get("avgPower").and_then(|v| v.as_i64()).unwrap_or(0),
        total_steps: raw.get("totalSteps").and_then(|v| v.as_i64()).unwrap_or(0),
        total_ascent: raw.get("totalAscent").and_then(|v| v.as_i64()).unwrap_or(0),
        total_descent: raw.get("totalDescent").and_then(|v| v.as_i64()).unwrap_or(0),
        avg_step_freq: raw.get("avgStepFreq").and_then(|v| v.as_i64()).unwrap_or(0),
        valid_dis: raw.get("validDis").and_then(|v| v.as_f64()).unwrap_or(0.0),
        valid_time: raw.get("validTime").and_then(|v| v.as_f64()).unwrap_or(0.0) as i64,
        address: raw.get("address").and_then(|v| v.as_str()).unwrap_or("").into(),
        status_info: raw.get("statusInfo").and_then(|v| v.as_str()).unwrap_or("").into(),
        complete: raw.get("complete").and_then(|v| v.as_bool()).unwrap_or(false),
        reason_list: reasons,
        start_time: raw.get("startTime").and_then(|v| v.as_i64()).unwrap_or(0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detail_deserialization_preserves_raw_values_and_empty_reasons() {
        let fixture = r#"{
            "rrid": 42, "uuid": "fixture", "sportType": 1,
            "totalTime": 600, "totalDis": 1234.0, "speed": 2.3456789,
            "calorie": 60, "avgPower": 0, "totalSteps": 1200,
            "totalAscent": 0, "totalDescent": 0, "avgStepFreq": 120,
            "validDis": 1234.0, "validTime": 600, "address": "fixture",
            "statusInfo": "原始状态 RAW_STATUS / 未翻译",
            "complete": false,
            "reasonList": [{"reason": "原始原因 RAW_REASON", "complete": false}],
            "startTime": 1700000000000
        }"#;
        for input in [fixture.to_owned(), fixture.replace(
            r#"[{"reason": "原始原因 RAW_REASON", "complete": false}]"#, "[]"
        )] {
            let original: Value = serde_json::from_str(&input).unwrap();
            let detail: RunDetail = serde_json::from_str(&input).unwrap();
            assert_eq!(detail.speed, 2.3456789);
            assert_eq!(detail.status_info, "原始状态 RAW_STATUS / 未翻译");
            assert_eq!(detail.reason_list.len(), original["reasonList"].as_array().unwrap().len());
            let output: Value = serde_json::from_str(&serde_json::to_string_pretty(&detail).unwrap()).unwrap();
            for key in ["speed", "statusInfo", "reasonList"] {
                assert_eq!(output[key], original[key], "字段 {key} 被改写");
            }
        }
    }

    #[test]
    fn test_detail_serializes_camel_case() {
        let d = RunDetail {
            rrid: 42,
            total_dis: 1234.0,
            valid_dis: 1200.0,
            status_info: "有效".into(),
            reason_list: vec![ReasonItem { reason: "里程达标".into(), complete: true }],
            start_time: 1_700_000_000_000,
            ..Default::default()
        };
        let v: serde_json::Value = serde_json::to_value(&d).unwrap();
        for key in ["totalDis", "validDis", "statusInfo", "reasonList", "startTime", "sportType", "avgStepFreq"] {
            assert!(v.get(key).is_some(), "缺少键 {key}");
        }
        assert!(v.get("total_dis").is_none());
    }
}
