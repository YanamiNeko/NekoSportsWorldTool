//! 校园点位：POST /api/v560/get/1/distance/1（sportType=4）。
//!
//! runec = 信封(f"{uid}{经度6位}{纬度6位}{起始时间秒级ms整}"，insert→observed 序列化)；
//! sign = MD5(http版URL + 盐)。带 300s 缓存（限流 10603：5 分钟 3 次），失败回退最近缓存。

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

/// 拉取实时点位（带缓存回退）。anchor=(lat,lng) 请求锚点。
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
    // ① TTL 内命中缓存直接返回
    if let Some((ts, pts)) = model::load_points_cache_for(anchor) {
        if !pts.is_empty()
            && crate::crypto::envelope::now_ms() - ts < model::POINTS_TTL_MS
        {
            log(&format!("[points] 缓存命中（{} 秒前，{} 点）", (crate::crypto::envelope::now_ms() - ts) / 1000, pts.len()));
            return Ok(pts);
        }
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
    let runec_input = format!("{uid}{}{}{}", six_digit(lon), six_digit(lat), (start_ms / 1000) * 1000);
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
    let fallback = |log: &mut dyn FnMut(&str)| -> Result<Vec<Value>, String> {
        if let Some((_ts, pts)) = model::load_points_cache_for(anchor) {
            if !pts.is_empty() {
                log("[points] 接口失败，回退最近一次接口结果缓存");
                return Ok(pts);
            }
        }
        Err("点位接口失败且无缓存".into())
    };
    let Some(dec) = out.decrypted else {
        return fallback(log);
    };
    let payload = &dec.business;
    // pointsResModels：优先顶层，其次 data 子对象
    let pts: Vec<Value> = super::client::get_field(payload, "pointsResModels")
        .and_then(|m| m.as_array())
        .cloned()
        .unwrap_or_default();
    if !pts.is_empty() {
        let _ = model::save_points_cache(anchor, &pts);
        return Ok(pts);
    }
    let err = payload.get("error").and_then(|e| e.as_i64()).unwrap_or(0);
    log(&format!(
        "[points] 接口无点位 error={err}: {}",
        payload.get("message").and_then(|m| m.as_str()).unwrap_or("")
    ));
    fallback(log)
}

/// 点位中心（BD 系）。
#[allow(dead_code)]
pub fn center_bd(points: &[Value]) -> (f64, f64) {
    let n = points.len().max(1) as f64;
    let lat = points.iter().filter_map(|p| p.get("lat").and_then(|v| v.as_f64())).sum::<f64>() / n;
    let lon = points.iter().filter_map(|p| p.get("lon").and_then(|v| v.as_f64())).sum::<f64>() / n;
    (lat, lon)
}

/// 点位 → (lat, lon) BD 系数组（轨迹输入）。
pub fn points_bd(points: &[Value]) -> Vec<(f64, f64)> {
    points
        .iter()
        .filter_map(|p| {
            let lat = p.get("lat")?.as_f64()?;
            let lon = p.get("lon")?.as_f64()?;
            Some((lat, lon))
        })
        .collect()
}

/// 点位 → BD 系 (lat, lon)，按 `isFixed` 分类。
fn points_by_fixed(points: &[Value], fixed: bool) -> Vec<(f64, f64)> {
    points
        .iter()
        .filter(|p| (p.get("isFixed").and_then(|v| v.as_i64()).unwrap_or(0) != 0) == fixed)
        .filter_map(|p| {
            let lat = p.get("lat")?.as_f64()?;
            let lon = p.get("lon")?.as_f64()?;
            Some((lat, lon))
        })
        .collect()
}

/// 必经点（`isFixed != 0`，BD 系）。
pub fn points_fixed(points: &[Value]) -> Vec<(f64, f64)> {
    points_by_fixed(points, true)
}

/// 普通打卡点（`isFixed == 0` 或缺失，BD 系）。
pub fn points_free(points: &[Value]) -> Vec<(f64, f64)> {
    points_by_fixed(points, false)
}

/// 由点位解析硬点，返回 `(必经点, 普通点)`：必经点 = `isFixed != 0` 的点；普通点 = `isFixed == 0`。
///
/// 预览与实际运行共用此函数，保证二者的选点输入一致。policy 响应不含必经点，
/// 硬点唯一来源就是点位接口的 `isFixed`。
pub fn resolve_hard_points(points: &[Value]) -> (Vec<(f64, f64)>, Vec<(f64, f64)>) {
    (points_fixed(points), points_free(points))
}

/// 由实际经过的硬点 `selected`（BD 系）构造提交/OBS 用的点位数组。
///
/// 命中 `points` 的沿用实时点位原始 `Value`；未命中的硬点合成一条：`isFixed=1`、
/// BD 原坐标、`glat/glon` 由 BD→GCJ 得到。这样 `fivePointJson` / OBS `fixed_point_json`
/// 与轨迹实际经过的硬点完全一致，避免「轨迹经过但点集缺失」。
pub fn submitted_points_for(selected: &[(f64, f64)], points: &[Value]) -> Vec<Value> {
    let near = |a: &(f64, f64), b: &(f64, f64)| (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6;
    selected
        .iter()
        .map(|q| {
            let hit = points.iter().find(|p| {
                match (
                    p.get("lat").and_then(|v| v.as_f64()),
                    p.get("lon").and_then(|v| v.as_f64()),
                ) {
                    (Some(lat), Some(lon)) => near(&(lat, lon), q),
                    _ => false,
                }
            });
            match hit {
                Some(p) => p.clone(),
                None => {
                    let (glat, glon) = crate::track::wire::bd09_to_gcj02(q.0, q.1);
                    json!({
                        "lat": q.0,
                        "lon": q.1,
                        "glat": glat,
                        "glon": glon,
                        "isFixed": 1,
                        "pointName": "",
                    })
                }
            }
        })
        .collect()
}
