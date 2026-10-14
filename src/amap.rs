//! 高德开放平台 Web 服务客户端（自定义路线用）。
//!
//! - 地址解析（geocode/geo）：地址文本 → gcj02 坐标；
//! - 步行路径规划（direction/walking）：两点间真实步行路线折线；
//! - 绕圈闭环：去程 A→B + 返程 B→A 合成自然闭环（返程失败回退去程反向）。
//!
//! 返回坐标均为 gcj02（高德系），进入轨迹生成前统一转 BD-09 对齐打卡点。
//! 仅使用用户配置的 key，不登录、不访问学校服务器。

use crate::api::client::ureq_err;
use crate::track::custom::{CustomRing, CustomRouteSpec};
use crate::track::geom::gcj02_to_bd09;

const GEO_URL: &str = "https://restapi.amap.com/v3/geocode/geo";
const WALK_URL: &str = "https://restapi.amap.com/v3/direction/walking";

fn http_get_json(url: &str, query: &[(&str, &str)]) -> Result<serde_json::Value, String> {
    let agent = ureq::AgentBuilder::new()
        .timeout(std::time::Duration::from_secs(20))
        .build();
    let mut req = agent.get(url);
    for (k, v) in query {
        req = req.query(k, v);
    }
    let resp = req.call().map_err(ureq_err)?;
    let body = resp.into_string().unwrap_or_default();
    serde_json::from_str::<serde_json::Value>(&body)
        .map_err(|e| format!("高德响应 JSON 解析失败: {e}（原文: {}）", crate::textlog::truncate(&body, 120)))
}

fn check_status(v: &serde_json::Value) -> Result<(), String> {
    let status = v.get("status").and_then(|s| s.as_str()).unwrap_or("");
    if status != "1" {
        let info = v.get("info").and_then(|s| s.as_str()).unwrap_or("未知错误");
        return Err(format!("高德返回错误：{info}"));
    }
    Ok(())
}

/// 全角标点归一化：中文输入法常把句点/逗号映射为全角，坐标解析前统一转半角。
fn normalize_punct(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '。' => '.',
            '，' => ',',
            '：' => ':',
            '；' => ';',
            '　' => ' ',
            _ => c,
        })
        .collect()
}

/// 解析坐标文本：支持「经度,纬度」与「纬度,经度」自动识别（境内按数值范围判定）。
fn parse_coords(s: &str) -> Option<(f64, f64)> {
    let s = normalize_punct(s);
    let s = s.trim();
    let (a, b) = s.split_once(',')?;
    let (a, b) = (a.trim().parse::<f64>().ok()?, b.trim().parse::<f64>().ok()?);
    let lng_range = 73.0..=136.0;
    let lat_range = 3.0..=54.0;
    if lng_range.contains(&a) && lat_range.contains(&b) {
        Some((b, a)) // (lat, lng)
    } else if lat_range.contains(&a) && lng_range.contains(&b) {
        Some((a, b))
    } else {
        None
    }
}

/// 解析起点/终点 → gcj02 (lat, lng)。纯地址文本走 geocoding。
pub fn resolve_to_gcj(spec: &str, key: &str) -> Result<(f64, f64), String> {
    let spec = spec.trim();
    if spec.is_empty() {
        return Err("地址为空".into());
    }
    if let Some((lat, lng)) = parse_coords(spec) {
        return Ok((lat, lng));
    }
    let v = http_get_json(GEO_URL, &[("address", spec), ("key", key)])?;
    check_status(&v)?;
    let geocodes = v.get("geocodes").and_then(|g| g.as_array()).ok_or("高德地址解析无结果")?;
    let location = geocodes
        .first()
        .and_then(|g| g.get("location"))
        .and_then(|l| l.as_str())
        .ok_or_else(|| format!("未找到地址「{spec}」"))?;
    parse_coords(location).ok_or_else(|| format!("高德返回的坐标格式异常: {location}"))
}

/// 兼容数字/字符串两种 JSON 数值形式（高德部分字段为字符串数字）。
fn as_f64_compat(v: &serde_json::Value) -> Option<f64> {
    match v {
        serde_json::Value::Number(n) => n.as_f64(),
        serde_json::Value::String(s) => s.trim().parse::<f64>().ok(),
        _ => None,
    }
}

/// 两点间步行路线折线（gcj02 (lat, lng)）与长度（米）。
pub fn walking_leg(
    from_gcj: (f64, f64),
    to_gcj: (f64, f64),
    key: &str,
) -> Result<(Vec<(f64, f64)>, f64), String> {
    let origin = format!("{:.7},{:.7}", from_gcj.1, from_gcj.0);
    let destination = format!("{:.7},{:.7}", to_gcj.1, to_gcj.0);
    let v = http_get_json(
        WALK_URL,
        &[("origin", origin.as_str()), ("destination", destination.as_str()), ("key", key)],
    )?;
    check_status(&v)?;
    let route = v.get("route").ok_or("高德步行规划响应缺少 route")?;
    let path = route
        .get("paths")
        .and_then(|p| p.as_array())
        .and_then(|a| a.first())
        .ok_or("高德步行规划无可用路线")?;
    let distance = path.get("distance").and_then(as_f64_compat).unwrap_or(0.0);
    let steps = path.get("steps").and_then(|s| s.as_array()).ok_or("路线缺少 steps")?;
    let mut pts: Vec<(f64, f64)> = Vec::new();
    for step in steps {
        let poly = step.get("polyline").and_then(|p| p.as_str()).unwrap_or("");
        for seg in poly.split(';') {
            if let Some((lat, lng)) = parse_coords(seg) {
                if pts.last() != Some(&(lat, lng)) {
                    pts.push((lat, lng));
                }
            }
        }
    }
    if pts.len() < 2 {
        return Err("路线折线点不足（<2）".into());
    }
    Ok((pts, distance))
}

/// 从高德拉取自定义路线：单程 A→B 或绕圈 A→B→A 闭环，输出 BD-09 环。
pub fn fetch_custom_ring(spec: &CustomRouteSpec, key: &str) -> Result<CustomRing, String> {
    if key.trim().is_empty() {
        return Err("未配置高德 API Key（跑步页填写或配置 amap_key）".into());
    }
    let o = resolve_to_gcj(&spec.origin, key)?;
    let d = resolve_to_gcj(&spec.destination, key)?;
    let same_point = (o.0 - d.0).abs() < 1e-6 && (o.1 - d.1).abs() < 1e-6;
    if same_point {
        return Err(
            "起点与终点相同：绕圈请选择两个不同地点（去程+返程合成一圈），或直接用经典打卡点环模式"
                .into(),
        );
    }
    let (leg1, leg1_len) = walking_leg(o, d, key)?;

    let (ring_gcj, total_len, loop_mode) = if spec.loop_mode {
        // 返程 B→A：高德给的返程路线通常与去程不同，合成更自然的闭环
        let leg2 = match walking_leg(d, o, key) {
            Ok((pts, len)) => (pts, len),
            Err(_) => {
                let mut rev = leg1.clone();
                rev.reverse();
                (rev, leg1_len)
            }
        };
        let (leg2_pts, leg2_len) = leg2;
        let mut ring = leg1.clone();
        ring.extend(leg2_pts.iter().skip(1));
        let n = ring.len();
        if n > 2 && ring[0] == ring[n - 1] {
            ring.pop();
        }
        (ring, leg1_len + leg2_len, true)
    } else {
        (leg1.clone(), leg1_len, false)
    };

    let points_bd: Vec<(f64, f64)> = ring_gcj
        .iter()
        .map(|&(la, lo)| gcj02_to_bd09(la, lo))
        .collect();
    Ok(CustomRing {
        points_bd,
        leg_length_m: leg1_len,
        total_length_m: total_len,
        loop_mode,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn coord_parsing_detects_both_orders() {
        assert_eq!(parse_coords("121.540,38.900"), Some((38.900, 121.540)));
        assert_eq!(parse_coords("38.900,121.540"), Some((38.900, 121.540)));
        // 中文输入法全角标点同样可解析
        assert_eq!(parse_coords("121。540，38。900"), Some((38.900, 121.540)));
        assert_eq!(parse_coords("abc"), None);
        assert_eq!(parse_coords("10,10"), None);
    }

    #[test]
    fn empty_key_is_rejected() {
        let spec = CustomRouteSpec::default();
        assert!(fetch_custom_ring(&spec, "").is_err());
    }
}
