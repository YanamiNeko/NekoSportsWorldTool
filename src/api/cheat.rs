//! 违规查询快照；缺字段不等于服务端明确返回 null。
use super::client::ApiClient;
use crate::api::model::DISCOVERY;
use serde_json::{json, Value};

pub const CHEATLIST_PATH: &str = "/api/v78/cheat/cheatlist";
pub const FRESH_MS: i64 = 5 * 60 * 1000;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct CheatReport {
    #[serde(rename = "self")]
    pub self_info: Value,
    pub list: Vec<Value>,
    #[serde(default)]
    pub self_present: bool,
    #[serde(default)]
    pub checked_at_ms: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    NoRecord,
    Flagged,
    Unknown,
    Stale,
}

impl CheatReport {
    pub fn from_data(data: &Value, checked_at_ms: i64) -> Result<Self, String> {
        if !data.is_object() {
            return Err("违规接口数据格式异常，无法判断".into());
        }
        Ok(Self {
            self_info: data.get("self").cloned().unwrap_or(Value::Null),
            self_present: data.get("self").is_some(),
            list: data
                .get("list")
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default(),
            checked_at_ms,
        })
    }
    pub fn status_at(&self, now: i64) -> Status {
        if self.checked_at_ms <= 0
            || !(0..FRESH_MS).contains(&now.saturating_sub(self.checked_at_ms))
        {
            return Status::Stale;
        }
        if !self.self_present {
            return Status::Unknown;
        }
        match &self.self_info {
            Value::Null => Status::NoRecord,
            Value::Object(o) if !o.is_empty() => Status::Flagged,
            _ => Status::Unknown,
        }
    }
    pub fn status(&self) -> Status {
        self.status_at(crate::crypto::envelope::now_ms())
    }
    pub fn label(&self) -> &'static str {
        match self.status() {
            Status::NoRecord => "本次查询暂未发现违规记录",
            Status::Flagged => "服务端已标记违规",
            Status::Unknown => "结果未知：个人状态字段缺失或格式异常",
            Status::Stale => "检查结果已过期，请重新检查",
        }
    }
    pub fn reason(&self) -> String {
        text_field(&self.self_info, &["reason", "punishReason", "cause"])
            .unwrap_or_else(|| "服务端未提供原因".into())
    }
    pub fn occurred_at(&self) -> String {
        let Some(text) = text_field(&self.self_info, &["createTime", "time", "date"]) else {
            return "服务端未提供时间".into();
        };
        if let Ok(n) = text.parse::<i64>() {
            let ms = if (1_000_000_000..10_000_000_000).contains(&n) {
                n * 1000
            } else {
                n
            };
            if (1_000_000_000_000..10_000_000_000_000).contains(&ms) {
                return crate::track::geom::fmt_cst(ms, "%Y-%m-%d %H:%M:%S 北京时间");
            }
        }
        text
    }
    pub fn record_id(&self) -> Option<i64> {
        // Never confuse a notice ID with a running-record ID.
        let id = self.self_info.get("rrid")?;
        id.as_i64()
            .or_else(|| id.as_str()?.parse().ok())
            .filter(|n| *n > 0)
    }
}
fn text_field(value: &Value, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| match value.get(key)? {
        Value::String(s) if !s.trim().is_empty() => Some(s.trim().into()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    })
}
pub fn query(
    client: &mut ApiClient,
    page: i64,
    log: &mut dyn FnMut(&str),
) -> Result<CheatReport, String> {
    let unid: i64 = client
        .login
        .as_ref()
        .map(|s| s.unid.parse().unwrap_or(0))
        .unwrap_or(0);
    let body = json!({ "pageNum": page, "pageSize": 20, "unid": unid }).to_string();
    let biz = client.call_host(DISCOVERY, "POST", CHEATLIST_PATH, &body, &[])?;
    let report = CheatReport::from_data(
        &super::client::parse_data_field(&biz),
        crate::crypto::envelope::now_ms(),
    )?;
    log(&format!(
        "[cheat] {}；本页通报 {} 条",
        report.label(),
        report.list.len()
    ));
    Ok(report)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_malformed_and_empty_self_never_mean_clean() {
        for data in [
            json!({}),
            json!({"self":{}}),
            json!({"self":[]}),
            json!({"self":false}),
            json!({"self":0}),
            json!({"self":"null"}),
        ] {
            assert_eq!(
                CheatReport::from_data(&data, 1000).unwrap().status_at(1000),
                Status::Unknown
            );
        }
        assert!(CheatReport::from_data(&Value::Null, 1000).is_err());
        assert_eq!(
            CheatReport::from_data(&json!({"self":null}), 1000)
                .unwrap()
                .status_at(1000),
            Status::NoRecord
        );
    }
    #[test]
    fn timestamp_bounds_and_message_roundtrip() {
        let r = CheatReport::from_data(&json!({"self":null}), 1000).unwrap();
        assert_eq!(r.status_at(999), Status::Stale);
        assert_eq!(r.status_at(1000 + FRESH_MS), Status::Stale);
        let copy: CheatReport = serde_json::from_value(serde_json::to_value(&r).unwrap()).unwrap();
        assert_eq!(copy.status_at(1000), Status::NoRecord);
        let old: CheatReport = serde_json::from_value(json!({"self":null,"list":[]})).unwrap();
        assert_eq!(old.status_at(1000), Status::Stale);
    }
    #[test]
    fn details_preserve_reason_and_only_use_explicit_record_id() {
        let r = CheatReport::from_data(&json!({"self":{"reason":"","punishReason":"原始原因","createTime":1_700_000_000,"rrid":"42","id":99}}), 1000).unwrap();
        assert_eq!(r.status_at(1000), Status::Flagged);
        assert_eq!(r.reason(), "原始原因");
        assert_eq!(r.record_id(), Some(42));
        assert_eq!(r.occurred_at(), "2023-11-15 06:13:20 北京时间");
        let missing = CheatReport::from_data(&json!({"self":{"id":99}}), 1000).unwrap();
        assert_eq!(missing.reason(), "服务端未提供原因");
        assert_eq!(missing.record_id(), None);
    }
}
