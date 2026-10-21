//! 校园点位：POST /api/v560/get/1/distance/1（sportType=4）。
//!
//! runec = 信封(f"{uid}{经度6位}{纬度6位}{起始时间秒级ms整}"，insert→observed 序列化)；
//! sign = MD5(http版URL + 盐)。带按账号、学校、区域及位置隔离的 300s 缓存。

use super::client::ApiClient;
use super::model;
use crate::crypto::envelope::{build_envelope, OuterOrder};
use crate::crypto::sign::md5_url_sign;
use crate::location::Coordinate;
use serde_json::{json, Value};

pub const POINTS_PATH: &str = "/api/v560/get/1/distance/1";

/// 经纬度六位小数字符串。
fn six_digit(v: f64) -> String {
    format!("{v:.6}")
}

/// 拉取实时点位（仅使用有效期内缓存）。anchor=(lat,lng) 请求锚点。
pub fn fetch_points(
    client: &mut ApiClient,
    anchor: Coordinate,
    log: &mut dyn FnMut(&str),
) -> Result<Vec<Value>, String> {
    fetch_points_ext(client, anchor, None, log)
}

/// run_area_id：学校配置了区域时 App 会附带；未配置则不传（与 App 一致）。
pub fn fetch_points_ext(
    client: &mut ApiClient,
    anchor: Coordinate,
    run_area_id: Option<String>,
    log: &mut dyn FnMut(&str),
) -> Result<Vec<Value>, String> {
    anchor.validate()?;
    let login = client.login.as_ref().ok_or("请先登录后获取点位")?;
    let scope = model::PointsCacheScope {
        uid: login.uid,
        unid: login.unid.clone(),
        run_area_id: run_area_id.clone(),
    };
    // ① TTL 内命中缓存直接返回
    if let Some((ts, pts)) = model::load_points_cache_for(anchor, &scope) {
        log(&format!(
            "[points] 缓存命中（{} 秒前，{} 点）",
            (crate::crypto::envelope::now_ms() - ts) / 1000,
            pts.len()
        ));
        return Ok(pts);
    }
    // ② 请求接口
    let uid = client.login.as_ref().map(|s| s.uid).unwrap_or(0);
    let unid = client
        .login
        .as_ref()
        .map(|s| s.unid.clone())
        .unwrap_or_else(|| "0".into());
    let lat = anchor.latitude;
    let lon = anchor.longitude;
    let url = format!("{}{}", model::HOST, POINTS_PATH);

    let start_ms = crate::crypto::envelope::now_ms();
    let runec_input = format!(
        "{uid}{}{}{}",
        six_digit(lon),
        six_digit(lat),
        (start_ms / 1000) * 1000
    );
    // runec：共用会话 Four，外层 observed 序
    let runec_env = build_envelope(&mut client.session, &runec_input, OuterOrder::Observed);
    let runec = runec_env.json;

    let mut body = json!({
        "sportType": 4,
        "longitude": lon,
        "latitude": lat,
        "sign": md5_url_sign(&url),
        "uuid": uuid::Uuid::new_v4().to_string(),
        "selectedUnid": unid,
        "runec": runec,
    });
    if let Some(area) = run_area_id {
        body["runAreaId"] = json!(area);
    }
    let body = body.to_string();

    let out = client.envelope_request("POST", &url, &body, crate::crypto::header::UA_IOS, &[])?;
    let dec = out.decrypted.ok_or("点位接口响应无法解密")?;
    let pts = parse_points(&dec.business)?;
    let _ = model::save_points_cache(anchor, &scope, &pts);
    Ok(pts)
}

pub(crate) fn parse_points(payload: &Value) -> Result<Vec<Value>, String> {
    let payload = super::client::check_business(payload)?;
    // pointsResModels：优先顶层，其次 data 子对象
    let pts: Vec<Value> = super::client::get_field(&payload, "pointsResModels")
        .and_then(|m| m.as_array())
        .cloned()
        .unwrap_or_default();
    if pts.is_empty() {
        return Err("点位接口未返回有效点位列表".into());
    }
    if points_bd(&pts).len() != pts.len() {
        return Err("点位接口含无效坐标".into());
    }
    Ok(pts)
}

/// 点位中心（BD 系）。
#[allow(dead_code)]
pub fn center_bd(points: &[Value]) -> (f64, f64) {
    let n = points.len().max(1) as f64;
    let lat = points
        .iter()
        .filter_map(|p| p.get("lat").and_then(|v| v.as_f64()))
        .sum::<f64>()
        / n;
    let lon = points
        .iter()
        .filter_map(|p| p.get("lon").and_then(|v| v.as_f64()))
        .sum::<f64>()
        / n;
    (lat, lon)
}

/// 点位 → (lat, lon) BD 系数组（轨迹输入）。
pub fn points_bd(points: &[Value]) -> Vec<(f64, f64)> {
    points.iter().filter_map(super::fence::point_xy).collect()
}

/// Preserve policy order, then add distinct live points in a stable radial order.
pub fn route_points(required: &[(f64, f64)], live: &[Value]) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    for p in required
        .iter()
        .copied()
        .chain(crate::track::generate_road::radial_order(&points_bd(live)))
    {
        if !out
            .iter()
            .any(|q: &(f64, f64)| (q.0 - p.0).abs() < 1e-6 && (q.1 - p.1).abs() < 1e-6)
        {
            out.push(p);
        }
    }
    out
}

#[cfg(test)]
mod route_tests {
    use super::*;
    #[test]
    fn direct_envelope_points_decode_json_text_business_data() {
        let point = json!({"lat":"39.9","lon":"116.4","isMustPoint":false});
        let data = json!({"pointsResModels":[point.clone()]});
        let response = json!({"error":10000,"data":data.to_string()});
        let points = parse_points(&response).unwrap();
        assert_eq!(points, vec![point]);
        assert_eq!(points_bd(&points), vec![(39.9, 116.4)]);
        assert!(parse_points(&json!({"error":10000,"data":"{broken"})).is_err());
    }

    #[test]
    fn response_errors_and_invalid_coordinates_are_not_cached_as_success() {
        let point = json!({"lat":39.9,"lon":116.4});
        assert_eq!(
            parse_points(&json!({"error":10000,"data":{"pointsResModels":[point.clone()]}}))
                .unwrap(),
            vec![point.clone()]
        );
        for response in [
            json!({"error":10603,"pointsResModels":[point]}),
            json!({"error":10000,"pointsResModels":[]}),
            json!({"error":10000,"pointsResModels":[{}]}),
        ] {
            assert!(parse_points(&response).is_err());
        }
    }
    #[test]
    fn first_required_point_and_order_survive_deduplication() {
        let first = (39.9, 116.4);
        let second = (39.91, 116.41);
        let live = vec![
            json!({"lat": 39.9,"lon":116.4}),
            json!({"lat":39.92,"lon":116.42}),
            json!({"lat":39.92,"lon":116.42}),
        ];
        assert_eq!(
            route_points(&[first, first, second], &live),
            vec![first, second, (39.92, 116.42)]
        );
    }
}
