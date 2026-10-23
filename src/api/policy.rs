//! 跑步策略：POST /api/v70103/runModePolicy。
//! 返回 data.runRuleModel.minDistance（提交时 selDistance 用它）与 data.policy。

use super::client::{get_field, ApiClient};
use super::fence::point_xy;
use serde_json::{json, Value};

pub const POLICY_PATH: &str = "/api/v70103/runModePolicy";

pub struct PolicyInfo {
    pub timestamp: i64,
    pub policy: i64,
    pub min_distance: i64,
    pub valid_time: i64,
    /// 必经点（BD 系，与打卡点同系），policy 响应里若有则返回。
    pub must_points: Vec<(f64, f64)>,
    pub free_run_enabled: bool,
    pub free_run_fence_required: bool,
}

impl PolicyInfo {
    pub fn validate_free_run(&self, distance: f64, has_fences: bool) -> Result<(), String> {
        if !self.free_run_enabled {
            return Err("学校策略未明确开启自由跑，请使用打卡点模式或确认学校规则".into());
        }
        if !self.must_points.is_empty() {
            return Err("当前策略包含必经点，不能使用无打卡点自由跑".into());
        }
        if self.free_run_fence_required && !has_fences {
            return Err("自由跑策略要求或未明确免除学校围栏，但服务端未返回围栏".into());
        }
        if distance < self.min_distance as f64 {
            return Err(format!("自由跑距离不足学校最低要求 {} 米", self.min_distance));
        }
        Ok(())
    }
}

/// 从 policy 响应 `data` 中防御式提取必经点列表（字段名不确定，逐个尝试）。
fn extract_must_points(v: &serde_json::Value) -> Result<Vec<(f64, f64)>, String> {
    let mut points = Vec::new();
    for name in [
        "pointList",
        "runPointList",
        "mustPointList",
        "passPointList",
        "mustPoints",
        "passPoints",
        "runPoints",
        "points",
        "nodeList",
        "checkPointList",
    ] {
        let Some(value) = get_field(v, name).filter(|v| !v.is_null()) else {
            continue;
        };
        let arr = value
            .as_array()
            .ok_or_else(|| format!("策略点位 {name} 不是列表"))?;
        for (index, value) in arr.iter().enumerate() {
            let point =
                point_xy(value).ok_or_else(|| format!("策略点位 {name}[{index}] 坐标无效"))?;
            if !points
                .iter()
                .any(|p: &(f64, f64)| (p.0 - point.0).abs() < 1e-6 && (p.1 - point.1).abs() < 1e-6)
            {
                points.push(point);
            }
        }
    }
    Ok(points)
}

/// body：{"runMode":1,"ruleUpdateTime":0,"geoFenceUpdateTime":0,"selectUnid":<unid>,"operateType":0}
pub fn fetch_policy(client: &mut ApiClient) -> Result<PolicyInfo, String> {
    let unid = client
        .login
        .as_ref()
        .map(|s| s.unid.clone())
        .unwrap_or_default();
    let select_unid = unid.parse::<i64>().unwrap_or(0);
    let body = json!({
        "runMode": 1,
        "ruleUpdateTime": 0,
        "geoFenceUpdateTime": 0,
        "selectUnid": select_unid,
        "operateType": 0,
    })
    .to_string();
    let biz = client.call("POST", POLICY_PATH, &body, &[])?;
    parse_response(&biz)
}

pub(crate) fn parse_response(biz: &Value) -> Result<PolicyInfo, String> {
    let biz = super::client::normalize_business_data(biz)?;
    let timestamp = get_field(&biz, "timestamp")
        .and_then(|t| t.as_i64())
        .ok_or("policy 响应缺 timestamp")?;
    let policy = get_field(&biz, "policy")
        .and_then(|t| t.as_i64())
        .unwrap_or(0);
    let rule = get_field(&biz, "runRuleModel")
        .cloned()
        .unwrap_or(Value::Null);
    Ok(PolicyInfo {
        timestamp,
        policy,
        min_distance: rule
            .get("minDistance")
            .and_then(|t| t.as_i64())
            .unwrap_or(1000),
        valid_time: rule.get("validTime").and_then(|t| t.as_i64()).unwrap_or(0),
        must_points: extract_must_points(&biz)?,
        free_run_enabled: rule.get("freedomRunCalculateSingle").and_then(Value::as_bool) == Some(true),
        free_run_fence_required: match rule.get("freedomElectronicFence") {
            Some(Value::Bool(required)) => *required,
            Some(value) => value.as_i64() != Some(0),
            None => true,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn json_text_policy_preserves_required_points_and_rules() {
        let data = json!({"timestamp":123,"policy":4,"runRuleModel":{"minDistance":2000,"validTime":600},"mustPoints":[{"lat":"39.9","lon":"116.4"}]});
        for response in [json!({"data":data}), json!({"data":data.to_string()})] {
            let policy = parse_response(&response).unwrap();
            assert_eq!(policy.timestamp, 123);
            assert_eq!(policy.policy, 4);
            assert_eq!(policy.min_distance, 2000);
            assert_eq!(policy.valid_time, 600);
            assert_eq!(policy.must_points, vec![(39.9, 116.4)]);
        }
        assert!(parse_response(&json!({"data":"{broken"})).is_err());
    }

    #[test]
    fn merges_all_recognized_lists_without_losing_first_point() {
        let a = json!({"lat":39.9,"lon":116.4});
        let b = json!({"lat":39.91,"lon":116.41});
        let v = json!({"data":{"pointList":[a.clone()],"mustPoints":[a,b]}});
        assert_eq!(
            extract_must_points(&v).unwrap(),
            vec![(39.9, 116.4), (39.91, 116.41)]
        );
        assert!(extract_must_points(&json!({"mustPoints":null}))
            .unwrap()
            .is_empty());
    }
    #[test]
    fn malformed_required_points_are_errors_not_omissions() {
        assert!(
            extract_must_points(&json!({"mustPoints":[{}, {"lat":39.9,"lon":116.4}]})).is_err()
        );
        assert!(extract_must_points(&json!({"mustPoints":"bad"})).is_err());
    }
}
