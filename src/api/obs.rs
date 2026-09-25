//! OBS 上传：POST /api/obs/temporary/url 换签名 URL → PUT JSON。

use super::client::{get_field, parse_data_field, ApiClient};
use flate2::read::GzDecoder;
use serde_json::{json, Value};
use std::io::Read;

pub const OBS_SIGN_PATH: &str = "/api/obs/temporary/url";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ObsSummary {
    pub route_points: usize,
    pub run_area_id: i64,
    pub show_fence: bool,
    pub fence_count: usize,
    pub fence_bytes: usize,
}

impl ObsSummary {
    pub fn is_expected(&self) -> bool {
        self.route_points > 0
            && self.run_area_id >= -1
            && self.show_fence
            && self.fence_count > 0
    }

    pub fn matches(&self, expected: &Self) -> bool {
        expected.is_expected() && self == expected
    }
}

/// 换取签名 URL。
pub fn sign_url(client: &mut ApiClient, method: &str, key: &str) -> Result<String, String> {
    let body = json!({
        "bucketName": "iydsj-hbase-hot",
        "objectKey": key,
        "method": method,
        "contentType": "application/json",
    })
    .to_string();
    let biz = client.call("POST", OBS_SIGN_PATH, &body, &[])?;
    // business.data 可能是 JSON 字符串 {"signedUrl": ...}
    let data = parse_data_field(&biz);
    let signed = data
        .get("signedUrl")
        .and_then(|v| v.as_str())
        .or_else(|| get_field(&biz, "signedUrl").and_then(|v| v.as_str()))
        .ok_or("OBS 签名响应缺 signedUrl")?;
    Ok(signed.to_string())
}

/// PUT 上传 OBS 对象。
pub fn put_object(signed_url: &str, payload: &[u8], log: &mut dyn FnMut(&str)) -> Result<(), String> {
    let agent = super::client::make_agent();
    let resp = agent
        .put(signed_url)
        .set("Content-Type", "application/json")
        .send_bytes(payload)
        .map_err(super::client::ureq_err)?;
    log(&format!("[obs] PUT {} -> {}", shorten(signed_url), resp.status()));
    Ok(())
}

/// 上传到两个 objectKey（详情页 + 兜底路径）。
pub fn upload_both_keys(
    client: &mut ApiClient,
    keys: &[String],
    payload: &[u8],
    log: &mut dyn FnMut(&str),
) -> usize {
    let mut ok = 0;
    for key in keys {
        match sign_url(client, "Put", key).and_then(|url| put_object(&url, payload, log)) {
            Ok(()) => ok += 1,
            Err(e) => log(&format!("[obs] PUT {key} 失败: {e}")),
        }
    }
    ok
}

fn shorten(url: &str) -> &str {
    // 只显示 objectKey 部分，避免日志过长
    let start = url.find("run_data").unwrap_or(0);
    let end = (start + 60).min(url.len());
    &url[start..end]
}

fn decode_gz_json(encoded: &str) -> Result<Value, String> {
    let compressed = crate::crypto::envelope::b64_decode(encoded)?;
    let mut decoder = GzDecoder::new(compressed.as_slice());
    let mut raw = String::new();
    decoder.read_to_string(&mut raw).map_err(|e| format!("gzip 解压失败: {e}"))?;
    serde_json::from_str(&raw).map_err(|e| format!("OBS JSON 无效: {e}"))
}

fn decode_object_field(obj: &Value, name: &str) -> Result<Value, String> {
    let encoded = obj
        .get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("OBS 缺少 {name}"))?;
    decode_gz_json(encoded)
}

fn value_as_i64(value: &Value) -> Option<i64> {
    value
        .as_i64()
        .or_else(|| value.as_u64().and_then(|number| i64::try_from(number).ok()))
        .or_else(|| value.as_f64().filter(|number| number.is_finite()).map(|number| number as i64))
        .or_else(|| value.as_str().and_then(|text| text.trim().parse().ok()))
}

fn value_as_bool(value: &Value) -> Option<bool> {
    value.as_bool().or_else(|| {
        value.as_str().and_then(|text| match text.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" => Some(true),
            "false" | "0" | "no" => Some(false),
            _ => None,
        })
    })
}

fn array_field(value: &Value, name: &str) -> Result<(usize, usize), String> {
    let parsed = match value {
        Value::String(text) => serde_json::from_str::<Value>(text).map_err(|e| format!("{name} JSON 无效: {e}"))?,
        other => other.clone(),
    };
    let items = parsed.as_array().ok_or_else(|| format!("{name} 不是数组"))?;
    let bytes = match value {
        Value::String(text) => text.len(),
        _ => parsed.to_string().len(),
    };
    Ok((items.len(), bytes))
}

pub fn summarize_object(obj: &Value) -> Result<ObsSummary, String> {
    let run = decode_object_field(obj, "run_data")?;
    let points_value = run
        .get("allLocJson")
        .ok_or("run_data 缺少 allLocJson")?;
    let points = match points_value {
        Value::String(text) => serde_json::from_str::<Value>(text).map_err(|e| format!("路线 JSON 无效: {e}"))?,
        other => other.clone(),
    };
    let route_points = points
        .as_array()
        .map(|items| items.len())
        .ok_or("路线 JSON 不是数组")?;
    if route_points == 0 {
        return Err("路线 JSON 为空".into());
    }

    let fixed = decode_object_field(obj, "fixed_point_json")?;
    let run_area_id = fixed
        .get("runAreaId")
        .and_then(value_as_i64)
        .ok_or("fixed_point_json 缺少有效 runAreaId")?;
    let show_fence = fixed
        .get("freedomShowFence")
        .and_then(value_as_bool)
        .ok_or("fixed_point_json 缺少有效 freedomShowFence")?;
    let fence_value = fixed
        .get("geoFencesJson")
        .ok_or("fixed_point_json 缺少 geoFencesJson")?;
    let (fence_count, fence_bytes) = array_field(fence_value, "geoFencesJson")?;

    Ok(ObsSummary {
        route_points,
        run_area_id,
        show_fence,
        fence_count,
        fence_bytes,
    })
}

/// 回读验证（GET 签名）。
#[allow(dead_code)]
pub fn fetch_object(client: &mut ApiClient, key: &str, log: &mut dyn FnMut(&str)) -> Result<Value, String> {
    let signed = sign_url(client, "Get", key)?;
    let agent = super::client::make_agent();
    let resp = agent.get(&signed).call().map_err(super::client::ureq_err)?;
    let text = resp.into_string().unwrap_or_default();
    log(&format!("[obs] GET 回读 {} 字节", text.len()));
    serde_json::from_str(&text).map_err(|e| format!("OBS 对象解析失败: {e}"))
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn summarizes_compressed_route_and_area() {
        let run = json!({"allLocJson": "[{\"gLat\":39.4,\"gLng\":116.2}]", "useZip": false});
        let fixed = json!({"runAreaId": 42, "freedomShowFence": true, "geoFencesJson": "[{\"lat\":1}]", "useZip": false});
        let obj = json!({"run_data": crate::track::wire::gz(run.to_string().as_bytes()), "fixed_point_json": crate::track::wire::gz(fixed.to_string().as_bytes())});
        let summary = summarize_object(&obj).unwrap();
        assert_eq!(summary.route_points, 1);
        assert_eq!(summary.run_area_id, 42);
        assert!(summary.show_fence);
        assert_eq!(summary.fence_count, 1);
        assert_eq!(summary.fence_bytes, 11);
        assert!(summary.is_expected());
        assert!(summary.matches(&summary));

        let mut truncated = summary;
        truncated.route_points -= 1;
        assert!(!truncated.matches(&summary));
    }

    #[test]
    fn reports_default_area_without_accepting_it_as_expected() {
        let run = json!({"allLocJson": "[{\"gLat\":39.4,\"gLng\":116.2}]", "useZip": false});
        let fixed = json!({"runAreaId": -1, "freedomShowFence": false, "geoFencesJson": "[]", "useZip": false});
        let obj = json!({"run_data": crate::track::wire::gz(run.to_string().as_bytes()), "fixed_point_json": crate::track::wire::gz(fixed.to_string().as_bytes())});
        let summary = summarize_object(&obj).unwrap();
        assert_eq!(summary.route_points, 1);
        assert!(!summary.is_expected());
    }

    #[test]
    fn accepts_server_fence_when_area_id_is_unspecified() {
        let run = json!({"allLocJson": "[{\"gLat\":39.4,\"gLng\":116.2}]", "useZip": false});
        let fence_json = "[{\"lat\":39.4,\"lon\":116.2}]";
        let fixed = json!({"runAreaId": -1, "freedomShowFence": true, "geoFencesJson": fence_json, "useZip": false});
        let obj = json!({"run_data": crate::track::wire::gz(run.to_string().as_bytes()), "fixed_point_json": crate::track::wire::gz(fixed.to_string().as_bytes())});
        let summary = summarize_object(&obj).unwrap();
        assert_eq!(summary.run_area_id, -1);
        assert!(summary.is_expected());
        assert!(summary.matches(&summary));
    }
}
