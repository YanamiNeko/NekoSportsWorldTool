//! 电子围栏：POST /api/v1/getGeoFenceForRun。
//! 返回 data.geoFences[]，每个围栏 points[] 为坐标点（与打卡点同系，BD-09）。
//!
//! 服务端点结构可能只填充 GCJ 字段（glat/glon）而把 BD 字段（lat/lon）置 0，
//! 故优先取 lat/lon，若为 0/缺失则回退 glat/glon 并做 GCJ→BD 转换，避免缓存全 0。

use super::client::{get_field, ApiClient};
use crate::track::geom::gcj02_to_bd09;
use serde_json::{json, Value};

pub const FENCE_PATH: &str = "/api/v1/getGeoFenceForRun";

pub struct GeoFenceResponse {
    pub polygons: Vec<Vec<(f64, f64)>>,
    pub area: crate::track::wire::RunAreaMeta,
}

/// 拉取围栏多边形列表（每个围栏一个闭合环，(lat, lon)，BD-09）。
pub fn fetch_geo_fence(client: &mut ApiClient) -> Result<Vec<Vec<(f64, f64)>>, String> {
    Ok(fetch_geo_fence_with_area(client)?.polygons)
}

/// 拉取围栏并保留服务端原始元数据，供 fixed_point_json/OBS 回显绿色边界。
pub fn fetch_geo_fence_with_area(client: &mut ApiClient) -> Result<GeoFenceResponse, String> {
    let body = json!({ "geoFenceUpdateTime": 0 }).to_string();
    let biz = client.call("POST", FENCE_PATH, &body, &[])?;
    let fences = get_field(&biz, "geoFences")
        .cloned()
        .or_else(|| {
            let data = super::client::parse_data_field(&biz);
            data.get("geoFences").cloned()
        })
        .and_then(|value| match value {
            Value::Array(items) => Some(items),
            Value::String(text) => serde_json::from_str(&text).ok(),
            _ => None,
        })
        .unwrap_or_default();

    let mut out: Vec<Vec<(f64, f64)>> = Vec::new();
    for f in &fences {
        let Some(pts) = f.get("points").and_then(|v| v.as_array()) else {
            continue;
        };
        let ring: Vec<(f64, f64)> = pts
            .iter()
            .filter_map(point_xy)
            .filter(|(lat, lon)| !(*lat == 0.0 && *lon == 0.0))
            .collect();
        if ring.len() >= 3 {
            out.push(ring);
        }
    }
    let mut area = crate::api::points::area_from_payload(&biz, &[]);
    // The standalone endpoint may return lon/glon/pointsNumber, while the
    // detail payload used by the official client uses lng/glng and the compact
    // fence-point shape.  Keep the server-provided geometry, but emit that
    // canonical shape so the detail map can recognize the green boundary.
    if !out.is_empty() {
        area.geo_fences_json = canonical_fences_json(&fences);
        area.freedom_show_fence = area.geo_fences_json != "[]";
    }
    Ok(GeoFenceResponse {
        polygons: out,
        area,
    })
}

fn canonical_fences_json(fences: &[Value]) -> String {
    let canonical: Vec<Value> = fences
        .iter()
        .filter_map(|fence| {
            let points = fence.get("points")?.as_array()?;
            let points: Vec<Value> = points
                .iter()
                .filter_map(|point| {
                    let mut output = serde_json::Map::new();
                    for (canonical, aliases) in [
                        ("lat", &["lat", "latitude"][..]),
                        ("lng", &["lng", "lon", "longitude"][..]),
                        ("glat", &["glat"][..]),
                        ("glng", &["glng", "glon"][..]),
                    ] {
                        if let Some(value) = aliases.iter().find_map(|name| point.get(*name)) {
                            output.insert(canonical.into(), value.clone());
                        }
                    }
                    (output.contains_key("lat") && output.contains_key("lng")).then_some(Value::Object(output))
                })
                .collect();
            if points.len() < 3 {
                return None;
            }
            let mut output = serde_json::Map::new();
            for name in ["id", "name"] {
                if let Some(value) = fence.get(name) {
                    output.insert(name.into(), value.clone());
                }
            }
            output.insert("points".into(), Value::Array(points));
            Some(Value::Object(output))
        })
        .collect();
    if canonical.is_empty() {
        "[]".into()
    } else {
        Value::Array(canonical).to_string()
    }
}

/// 取一个点的 (lat, lon)（BD-09）。优先 lat/lon，缺失或全 0 时回退 glat/glon（GCJ→BD）。
/// 供围栏与 policy 必经点共用（两端点均实测存在「只填 GCJ、BD 置 0」的情况）。
pub(crate) fn point_xy(p: &Value) -> Option<(f64, f64)> {
    let lat = field_num(p, &["lat", "latitude"]);
    let lon = field_num(p, &["lon", "lng", "longitude"]);
    match (lat, lon) {
        (Some(a), Some(o)) if !(a == 0.0 && o == 0.0) => Some((a, o)),
        _ => {
            let (ga, go) = (field_num(p, &["glat"]), field_num(p, &["glon"]));
            match (ga, go) {
                (Some(a), Some(o)) if !(a == 0.0 && o == 0.0) => Some(gcj02_to_bd09(a, o)),
                _ => None,
            }
        }
    }
}

fn field_num(p: &Value, keys: &[&str]) -> Option<f64> {
    keys.iter().find_map(|k| p.get(*k).and_then(|v| v.as_f64()))
}

#[cfg(test)]
mod tests {
    use super::{canonical_fences_json, point_xy};
    use serde_json::{json, Value};

    #[test]
    fn point_xy_prefers_bd_then_gcj_fallback() {
        // BD lat/lon 正常
        let p = json!({ "lat": 38.9, "lon": 121.5 });
        assert_eq!(point_xy(&p), Some((38.9, 121.5)));
        // 经纬度字段用 lng
        let p = json!({ "lat": 38.9, "lng": 121.5 });
        assert_eq!(point_xy(&p), Some((38.9, 121.5)));
        // BD 全 0，回退 glat/glon（GCJ）并转 BD
        let p =
            json!({ "lat": 0.0, "lon": 0.0, "glat": 38.8956025774013, "glon": 121.5337497718317 });
        let (lat, lon) = point_xy(&p).unwrap();
        assert!((lat - 38.901678).abs() < 1e-6, "lat={lat}");
        assert!((lon - 121.540241).abs() < 1e-6, "lon={lon}");
        // 全缺
        let p = json!({ "foo": 1 });
        assert_eq!(point_xy(&p), None);
    }

    #[test]
    fn canonical_fence_uses_detail_map_coordinate_names() {
        let value = canonical_fences_json(&[json!({
            "id": 1774,
            "name": "学校",
            "points": [
                {"lat": 39.4, "lon": 116.2, "glat": 39.3, "glon": 116.1},
                {"lat": 39.4, "lon": 116.3, "glat": 39.3, "glon": 116.2},
                {"lat": 39.5, "lon": 116.3, "glat": 39.4, "glon": 116.2}
            ]
        })]);
        let parsed: Value = serde_json::from_str(&value).unwrap();
        assert_eq!(parsed[0]["id"], 1774);
        assert_eq!(parsed[0]["points"][0]["lng"], 116.2);
        assert_eq!(parsed[0]["points"][0]["glng"], 116.1);
        assert!(parsed[0]["points"][0].get("lon").is_none());
    }
}
